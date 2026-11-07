//! The microphone path (docs/ARQUITETURA.md, section 5): a plain WASAPI capture in shared mode
//! (Jay-C's reserve opens the same mic at the same time), WebRTC's audio processing, the user's
//! effect chain and the fixed output limiter, in 10 ms frames; handed out in 20 ms chunks.
//! Adapted from Jay-C's win.rs (`Mic`, `run_mic`, `list_mics`, `open_mic`).
//!
//! The capture thread is the audio thread: after the start it allocates nothing and never waits
//! on a lock (RNF-3).
//!
//! While Jay-C uses the mic, what Jay-C plays is the echo reference (echo.rs), and WebRTC's echo
//! canceller runs.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

use serde::{Deserialize, Serialize};
use wasapi::{AudioCaptureClient, AudioClient, DeviceEnumerator, Direction, Handle, SampleType, StreamMode, WaveFormat, initialize_mta};

use crate::apm::Apm;
use crate::dsp::chain::{ChainRunner, MAX_SLOTS};
use crate::echo::{EchoReference, Played, webview_of};
use crate::dsp::{Meters, output_limiter};
use crate::monitor::Monitor;
use crate::{CHUNK_FRAMES, SAMPLE_RATE, level_db};

/// The microphone and its WebRTC cleanup (echo cancellation comes with Jay-C: see `Mic::start`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicOptions {
    /// Remove steady background noise (fans, hum). Off is better for music.
    #[serde(default = "yes")]
    pub noise_suppression: bool,
    /// Level the voice automatically.
    #[serde(default = "yes")]
    pub auto_gain_control: bool,
    /// Capture endpoint id from `list_mics`; none (or one no longer present) = the Windows default.
    #[serde(default)]
    pub device: Option<String>,
}

fn yes() -> bool {
    true
}

impl Default for MicOptions {
    fn default() -> Self {
        MicOptions { noise_suppression: true, auto_gain_control: true, device: None }
    }
}

/// A microphone the user can pick. `id` is the WASAPI endpoint id, the same one Jay-C uses.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// A running microphone; dropping it closes the mic (and the monitor).
pub struct Mic {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    monitor: Arc<Mutex<Option<Monitor>>>,
    _echo: Option<EchoReference>,
    /// The open microphone's name and endpoint id (the default's when the chosen one is missing).
    pub device: String,
    pub device_id: String,
    /// Echo cancellation runs (Jay-C's WebView2 was found and is being captured).
    pub echo_cancellation: bool,
}

impl Drop for Mic {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.monitor.lock().unwrap().take();
    }
}

impl Mic {
    /// Opens the mic, running `chain` on it. `echo_of`: Jay-C's process, while Jay-C uses the mic;
    /// what its WebView2 plays is then removed from the mic. `on_chunk` gets each 20 ms chunk and
    /// the levels, on the audio thread (it must not allocate or wait: e.g. `try_send` to another
    /// thread); `on_error` a failure after the start (e.g. the mic was unplugged; the capture ends).
    pub fn start(
        options: MicOptions,
        echo_of: Option<u32>,
        chain: ChainRunner,
        on_chunk: impl FnMut(&[f32; CHUNK_FRAMES], &Meters) + Send + 'static,
        on_error: impl Fn(String) + Send + 'static,
    ) -> Result<Mic, String> {
        let echo = echo_of.and_then(|pid| {
            let Some(root) = webview_of(pid) else {
                log::warn!("no WebView2 under process {pid}: echo cancellation stays off");
                return None;
            };
            EchoReference::start(root).inspect_err(|e| log::warn!("echo reference: {e}")).ok()
        });
        let played = echo.as_ref().map(|e| e.played.clone());
        let stop = Arc::new(AtomicBool::new(false));
        let monitor: Arc<Mutex<Option<Monitor>>> = Arc::default();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("jay-audio-mic".into())
            .spawn({
                let (stop, monitor) = (stop.clone(), monitor.clone());
                move || run_mic(options, played, chain, &stop, &monitor, ready_tx, on_chunk, on_error)
            })
            .map_err(|e| e.to_string())?;
        match ready_rx.recv() {
            Ok(Ok((device, device_id))) => {
                Ok(Mic { stop, thread: Some(thread), monitor, echo_cancellation: echo.is_some(), _echo: echo, device, device_id })
            }
            Ok(Err(error)) => Err(error),
            Err(_) => Err("microphone thread exited during start".into()),
        }
    }

    /// "Hear my microphone": the cleaned mic on the default output device. For headphones only;
    /// on speakers the mic picks it up again.
    pub fn set_monitor(&self, on: bool) -> Result<(), String> {
        let mut monitor = self.monitor.lock().unwrap();
        if !on {
            monitor.take();
        } else if monitor.is_none() {
            *monitor = Some(Monitor::start()?);
        }
        Ok(())
    }
}

/// Active capture devices, for the microphone picker. Call from a thread that may init COM.
pub fn list_mics() -> Result<Vec<MicDevice>, String> {
    initialize_mta().ok().map_err(|e| e.to_string())?;
    let enumerator = DeviceEnumerator::new().map_err(|e| e.to_string())?;
    let default_id = enumerator.get_default_device(&Direction::Capture).and_then(|d| d.get_id()).ok();
    let collection = enumerator.get_device_collection(&Direction::Capture).map_err(|e| e.to_string())?;
    let mut mics = Vec::new();
    for device in &collection {
        let Ok(device) = device else { continue };
        let Ok(id) = device.get_id() else { continue };
        mics.push(MicDevice {
            is_default: default_id.as_deref() == Some(id.as_str()),
            name: device.get_friendlyname().unwrap_or_else(|_| id.clone()),
            id,
        });
    }
    Ok(mics)
}

/// Name, endpoint id and the started stream.
type OpenMic = (String, String, AudioClient, AudioCaptureClient, Handle);

/// Opens `device` (an endpoint id), or the default microphone when it's none or can't be opened
/// (unplugged, disabled).
fn open_mic(device: Option<&str>) -> Result<OpenMic, String> {
    let enumerator = DeviceEnumerator::new().map_err(|e| format!("no microphone: {e}"))?;
    if let Some(id) = device {
        match enumerator.get_device(id).map_err(|e| e.to_string()).and_then(open_device) {
            Ok(mic) => return Ok(mic),
            Err(e) => log::warn!("microphone {id} unavailable ({e}), using the default"),
        }
    }
    let default = enumerator.get_default_device(&Direction::Capture).map_err(|e| format!("no microphone: {e}"))?;
    open_device(default)
}

fn open_device(device: wasapi::Device) -> Result<OpenMic, String> {
    let name = device.get_friendlyname().unwrap_or_default();
    let id = device.get_id().unwrap_or_default();
    let mut client = device.get_iaudioclient().map_err(|e| e.to_string())?;
    let format = WaveFormat::new(32, 32, &SampleType::Float, SAMPLE_RATE as usize, 1, None);
    client
        .initialize_client(&format, &Direction::Capture, &StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 })
        .map_err(|e| format!("can't open the microphone: {e}"))?;
    let event = client.set_get_eventhandle().map_err(|e| e.to_string())?;
    let capture = client.get_audiocaptureclient().map_err(|e| e.to_string())?;
    client.start_stream().map_err(|e| e.to_string())?;
    Ok((name, id, client, capture, event))
}

#[allow(clippy::too_many_arguments)]
fn run_mic(
    options: MicOptions,
    played: Option<Played>,
    mut chain: ChainRunner,
    stop: &AtomicBool,
    monitor: &Mutex<Option<Monitor>>,
    ready: mpsc::Sender<Result<(String, String), String>>,
    mut on_chunk: impl FnMut(&[f32; CHUNK_FRAMES], &Meters),
    on_error: impl Fn(String),
) {
    crate::audio_thread_priority();
    if let Err(e) = initialize_mta().ok() {
        let _ = ready.send(Err(e.to_string()));
        return;
    }
    let (name, id, client, capture, event) = match open_mic(options.device.as_deref()) {
        Ok(mic) => mic,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let Some(mut apm) = Apm::new(SAMPLE_RATE, 1, 1, played.is_some(), options.noise_suppression, options.auto_gain_control) else {
        let _ = ready.send(Err("couldn't start audio processing".into()));
        return;
    };
    let _ = ready.send(Ok((name, id)));

    let frame = apm.capture_frame_len();
    let mut render = vec![0.0f32; apm.render_frame_len()];
    let mut limiter = output_limiter();
    // Everything the loop needs, allocated now: a second of room in each queue.
    let mut raw: VecDeque<u8> = VecDeque::with_capacity(SAMPLE_RATE as usize * 4);
    let mut pending: VecDeque<f32> = VecDeque::with_capacity(SAMPLE_RATE as usize);
    let mut samples = vec![0.0f32; frame];
    let mut chunk = [0.0f32; CHUNK_FRAMES];
    let mut filled = 0;
    let mut meters = Meters { level: f32::NEG_INFINITY, effects: [(0.0, 0.0); MAX_SLOTS], count: 0 };

    while !stop.load(Ordering::SeqCst) {
        if event.wait_for_event(100).is_err() {
            continue;
        }
        loop {
            match capture.get_next_packet_size() {
                Ok(Some(frames)) if frames > 0 => {
                    if let Err(e) = capture.read_from_device_to_deque(&mut raw) {
                        on_error(format!("microphone failed: {e}"));
                        return;
                    }
                }
                Ok(_) => break,
                Err(e) => {
                    on_error(format!("microphone failed: {e}"));
                    return;
                }
            }
        }
        let mut bytes = raw.drain(..raw.len() / 4 * 4);
        while let (Some(a), Some(b), Some(c), Some(d)) = (bytes.next(), bytes.next(), bytes.next(), bytes.next()) {
            pending.push_back(f32::from_le_bytes([a, b, c, d]));
        }
        drop(bytes);

        while pending.len() >= frame {
            // The echo canceller first gets everything Jay-C played since the last frame. A busy
            // lock (the reference thread adding to it) just leaves it for the next frame.
            if let Some(played) = &played
                && let Ok(mut queue) = played.try_lock()
            {
                let n = render.len();
                while queue.len() >= n {
                    for (r, p) in render.iter_mut().zip(queue.drain(..n)) {
                        *r = p;
                    }
                    apm.process_render(&render);
                }
            }
            for (s, p) in samples.iter_mut().zip(pending.drain(..frame)) {
                *s = p;
            }
            apm.process_capture(&mut samples, 0); // AEC3 estimates the speaker->mic delay itself
            chain.process(&mut samples);
            limiter.process(&mut samples);
            chunk[filled..filled + frame].copy_from_slice(&samples);
            filled += frame;
            if filled == CHUNK_FRAMES {
                filled = 0;
                // Never wait for the lock: turning the monitor on (opening a device) holds it a while.
                if let Ok(monitor) = monitor.try_lock()
                    && let Some(monitor) = monitor.as_ref()
                {
                    monitor.push(&chunk);
                }
                meters.level = level_db(&chunk);
                meters.count = 0;
                for (slot, peaks) in meters.effects.iter_mut().zip(chain.meters()) {
                    *slot = peaks;
                    meters.count += 1;
                }
                on_chunk(&chunk, &meters);
            }
        }
    }
    let _ = client.stop_stream();
}
