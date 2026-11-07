//! The soundboard in the user's headphones (or speakers): a stereo WASAPI render on the default
//! output whose thread mixes the voices as the device asks for them (it is the clock: no queue).
//! What it plays also goes, in mono, to `echo`: while Jay-C uses the mic, the echo canceller
//! removes it from the mic, so on speakers the clip isn't heard twice in the call. It closes
//! itself after a few seconds without voices; `Playback::running` says when to open a new one.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat, initialize_mta};

use crate::echo::Played;
use crate::soundboard::Mixer;
use crate::{CHUNK_FRAMES, SAMPLE_RATE};

/// Without voices for this long, the output closes (no audio session left open for nothing).
const IDLE: Duration = Duration::from_secs(5);
/// What the echo canceller hasn't taken yet; more (no mic running) and the oldest goes.
const ECHO_MAX: usize = CHUNK_FRAMES * 5;

pub struct Playback {
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Playback {
    /// Opens the default output and plays `mixer` on it, copying the mono mix into `echo`.
    pub fn start(mixer: Mixer, echo: Played) -> Result<Playback, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("jay-audio-soundboard".into())
            .spawn({
                let (stop, running) = (stop.clone(), running.clone());
                move || {
                    run(mixer, &echo, &stop, ready_tx);
                    running.store(false, Ordering::SeqCst);
                }
            })
            .map_err(|e| e.to_string())?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Playback { stop, running, thread: Some(thread) }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err("soundboard output thread exited during start".into()),
        }
    }

    /// False once it closed itself (idle) or the device failed.
    pub fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

/// The queue the echo canceller reads the soundboard from.
pub fn echo_queue() -> Played {
    Arc::new(Mutex::new(VecDeque::with_capacity(ECHO_MAX + CHUNK_FRAMES * 2)))
}

fn run(mut mixer: Mixer, echo: &Mutex<VecDeque<f32>>, stop: &AtomicBool, ready: mpsc::Sender<Result<(), String>>) {
    crate::audio_thread_priority();
    let opened = (|| {
        initialize_mta().ok().map_err(|e| e.to_string())?;
        let device = DeviceEnumerator::new()
            .and_then(|e| e.get_default_device(&Direction::Render))
            .map_err(|e| format!("no output device: {e}"))?;
        let mut client = device.get_iaudioclient().map_err(|e| e.to_string())?;
        let format = WaveFormat::new(32, 32, &SampleType::Float, SAMPLE_RATE as usize, 2, None);
        client
            .initialize_client(&format, &Direction::Render, &StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 })
            .map_err(|e| format!("can't open the output device: {e}"))?;
        let event = client.set_get_eventhandle().map_err(|e| e.to_string())?;
        let render = client.get_audiorenderclient().map_err(|e| e.to_string())?;
        let buffer = client.get_buffer_size().map_err(|e| e.to_string())? as usize;
        client.start_stream().map_err(|e| e.to_string())?;
        Ok((client, render, event, buffer))
    })();
    let (client, render, event, buffer) = match opened {
        Ok(stream) => stream,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let _ = ready.send(Ok(()));

    // Allocated once, for the most the device ever asks for.
    let mut mix = vec![0.0f32; buffer * 2];
    let mut bytes: Vec<u8> = Vec::with_capacity(buffer * 8);
    let mut last_voice = Instant::now();
    while !stop.load(Ordering::SeqCst) && last_voice.elapsed() < IDLE {
        if event.wait_for_event(100).is_err() {
            continue;
        }
        let frames = match client.get_available_space_in_frames() {
            Ok(frames) => (frames as usize).min(buffer),
            Err(e) => {
                log::warn!("soundboard output stopped: {e}");
                break;
            }
        };
        let mix = &mut mix[..frames * 2];
        mix.fill(0.0);
        mixer.mix(mix, 2);
        if mixer.voices() > 0 {
            last_voice = Instant::now();
            if let Ok(mut queue) = echo.try_lock() {
                queue.extend(mix.as_chunks::<2>().0.iter().map(|[l, r]| (l + r) * 0.5));
                if queue.len() > ECHO_MAX {
                    let excess = queue.len() - CHUNK_FRAMES * 2;
                    queue.drain(..excess);
                }
            }
        }
        bytes.clear();
        bytes.extend(mix.iter().flat_map(|s| s.to_le_bytes()));
        if let Err(e) = render.write_to_device(frames, &bytes, None) {
            log::warn!("soundboard output stopped: {e}");
            break;
        }
    }
    let _ = client.stop_stream();
}
