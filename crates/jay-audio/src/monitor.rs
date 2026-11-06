//! "Hear my microphone": the cleaned mic on the default output device (WASAPI render, shared mode).
//! Meant for headphones: on speakers the mic picks itself up again (feedback); the UI says so.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat, initialize_mta};

use crate::{CHUNK_FRAMES, SAMPLE_RATE};

/// Playback starts (and restarts after running dry) once this much is queued: the mic hands out
/// 20 ms chunks while the output asks for ~10 ms at a time, so playing from an empty queue would
/// crackle.
const START_QUEUED: usize = CHUNK_FRAMES * 2;
/// More than this queued (the mic's and the output's clocks drift apart) drops back to
/// START_QUEUED, like Jay-C's capture queues (`MAX_QUEUED_CHUNKS`).
const MAX_QUEUED: usize = CHUNK_FRAMES * 5;

type Queue = Arc<Mutex<VecDeque<f32>>>;

pub struct Monitor {
    queue: Queue,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Monitor {
    pub fn start() -> Result<Monitor, String> {
        let queue = Queue::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("jay-audio-monitor".into())
            .spawn({
                let (queue, stop) = (queue.clone(), stop.clone());
                move || run_monitor(&queue, &stop, ready_tx)
            })
            .map_err(|e| e.to_string())?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Monitor { queue, stop, thread: Some(thread) }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err("monitor thread exited during start".into()),
        }
    }

    /// Queues a mono chunk to play.
    pub fn push(&self, chunk: &[f32]) {
        let mut queue = self.queue.lock().unwrap();
        queue.extend(chunk);
        if queue.len() > MAX_QUEUED {
            let excess = queue.len() - START_QUEUED;
            queue.drain(..excess);
        }
    }
}

fn run_monitor(queue: &Mutex<VecDeque<f32>>, stop: &AtomicBool, ready: mpsc::Sender<Result<(), String>>) {
    let opened = (|| {
        initialize_mta().ok().map_err(|e| e.to_string())?;
        let device = DeviceEnumerator::new()
            .and_then(|e| e.get_default_device(&Direction::Render))
            .map_err(|e| format!("no output device: {e}"))?;
        let mut client = device.get_iaudioclient().map_err(|e| e.to_string())?;
        let format = WaveFormat::new(32, 32, &SampleType::Float, SAMPLE_RATE as usize, 1, None);
        client
            .initialize_client(&format, &Direction::Render, &StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 })
            .map_err(|e| format!("can't open the output device: {e}"))?;
        let event = client.set_get_eventhandle().map_err(|e| e.to_string())?;
        let render = client.get_audiorenderclient().map_err(|e| e.to_string())?;
        client.start_stream().map_err(|e| e.to_string())?;
        Ok((client, render, event))
    })();
    let (client, render, event) = match opened {
        Ok(stream) => stream,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let _ = ready.send(Ok(()));

    let mut playing = false;
    let mut bytes: Vec<u8> = Vec::new();
    while !stop.load(Ordering::SeqCst) {
        if event.wait_for_event(100).is_err() {
            continue;
        }
        let frames = match client.get_available_space_in_frames() {
            Ok(frames) => frames as usize,
            Err(e) => {
                log::warn!("monitor stopped: {e}");
                break;
            }
        };
        bytes.clear();
        {
            let mut queue = queue.lock().unwrap();
            playing = if playing { queue.len() >= frames } else { queue.len() >= START_QUEUED };
            if playing {
                bytes.extend(queue.drain(..frames).flat_map(f32::to_le_bytes));
            }
        }
        bytes.resize(frames * 4, 0); // silence while (re)filling
        if let Err(e) = render.write_to_device(frames, &bytes, None) {
            log::warn!("monitor stopped: {e}");
            break;
        }
    }
    let _ = client.stop_stream();
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::time::{Duration, Instant};

    use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat, initialize_mta};

    use super::Monitor;
    use crate::{CHUNK_FRAMES, SAMPLE_RATE, level_db};

    /// Level (dBFS) of what the default output plays during `duration`, read from its loopback,
    /// while `during` runs every 20 ms.
    fn output_level(duration: Duration, mut during: impl FnMut()) -> f32 {
        initialize_mta().ok().unwrap();
        let device = DeviceEnumerator::new().unwrap().get_default_device(&Direction::Render).unwrap();
        let mut client = device.get_iaudioclient().unwrap();
        let format = WaveFormat::new(32, 32, &SampleType::Float, SAMPLE_RATE as usize, 1, None);
        // Capture on a render device = its loopback.
        client.initialize_client(&format, &Direction::Capture, &StreamMode::PollingShared { autoconvert: true, buffer_duration_hns: 1_000_000 }).unwrap();
        let capture = client.get_audiocaptureclient().unwrap();
        client.start_stream().unwrap();
        let mut raw = VecDeque::new();
        let start = Instant::now();
        while start.elapsed() < duration {
            during();
            std::thread::sleep(Duration::from_millis(20));
            capture.read_from_device_to_deque(&mut raw).unwrap();
        }
        client.stop_stream().unwrap();
        let bytes: Vec<u8> = raw.into_iter().collect();
        let samples: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        level_db(&samples[samples.len() / 2..]) // the second half: past the monitor's start-up
    }

    #[test]
    #[ignore = "plays a quiet 440 Hz tone on the default output for ~2 s: cargo test -- --ignored"]
    fn monitor_plays_what_it_is_given() {
        let silence = output_level(Duration::from_millis(600), || {});
        let monitor = Monitor::start().expect("monitor");
        let mut phase = 0usize;
        let tone = output_level(Duration::from_millis(2000), || {
            let chunk: Vec<f32> = (phase..phase + CHUNK_FRAMES)
                .map(|i| 0.05 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / SAMPLE_RATE as f32).sin())
                .collect();
            phase += CHUNK_FRAMES;
            monitor.push(&chunk);
        });
        drop(monitor);
        println!("default output: {silence:.1} dBFS before, {tone:.1} dBFS with the monitor's tone (-29 dBFS sent)");
        assert!(tone > -40.0 && tone > silence + 15.0, "the tone didn't come out ({silence:.1} -> {tone:.1} dBFS)");
    }
}
