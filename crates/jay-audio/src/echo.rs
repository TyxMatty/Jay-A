//! The echo reference while Jay-C uses the microphone: what Jay-C plays (the call's voices),
//! captured from its WebView2 by process loopback, for WebRTC's echo canceller. Adapted from
//! Jay-C's win.rs (`process_table`, `own_audio_root`, `run_stream` with `Role::Feed`).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

use wasapi::{AudioClient, Direction, SampleType, StreamMode, WaveFormat, initialize_mta};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};

use crate::{CHUNK_FRAMES, SAMPLE_RATE};

/// What was played and not yet given to the echo canceller (mono, 48 kHz).
pub(crate) type Played = Arc<Mutex<VecDeque<f32>>>;

/// More than this waiting (the mic stalled) and the queue drops back to 40 ms, like Jay-C's.
const MAX_QUEUED: usize = CHUNK_FRAMES * 5;

/// pid → (parent pid, exe name).
fn process_table() -> HashMap<u32, (u32, String)> {
    let mut table = HashMap::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return table;
        };
        let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                table.insert(entry.th32ProcessID, (entry.th32ParentProcessID, String::from_utf16_lossy(&entry.szExeFile[..len])));
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    table
}

/// The WebView2 that `pid` (Jay-C) started: its tree plays the call. Not Jay-C's whole tree: a
/// Jay-A started by Jay-C is in it, and our own monitor would become part of the reference.
pub(crate) fn webview_of(pid: u32) -> Option<u32> {
    process_table()
        .into_iter()
        .filter(|(_, (parent, exe))| *parent == pid && exe.eq_ignore_ascii_case("msedgewebview2.exe"))
        .map(|(child, _)| child)
        .min() // normally exactly one; min() keeps the choice stable
}

/// Captures `root`'s process tree into a queue for the mic thread, until dropped.
pub(crate) struct EchoReference {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    pub played: Played,
}

impl Drop for EchoReference {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl EchoReference {
    pub fn start(root: u32) -> Result<EchoReference, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let played: Played = Arc::new(Mutex::new(VecDeque::with_capacity(MAX_QUEUED * 2)));
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("jay-audio-echo".into())
            .spawn({
                let (stop, played) = (stop.clone(), played.clone());
                move || run(root, &played, &stop, ready_tx)
            })
            .map_err(|e| e.to_string())?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(EchoReference { stop, thread: Some(thread), played }),
            Ok(Err(error)) => Err(error),
            Err(_) => Err("echo reference thread exited during start".into()),
        }
    }
}

fn run(root: u32, played: &Mutex<VecDeque<f32>>, stop: &AtomicBool, ready: mpsc::Sender<Result<(), String>>) {
    let opened = initialize_mta().ok().map_err(|e| e.to_string()).and_then(|()| {
        let format = WaveFormat::new(32, 32, &SampleType::Float, SAMPLE_RATE as usize, 1, None);
        let mut client = AudioClient::new_application_loopback_client(root, true).map_err(|e| format!("can't capture process {root}: {e}"))?;
        client
            .initialize_client(&format, &Direction::Capture, &StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 })
            .map_err(|e| format!("can't start capture of process {root}: {e}"))?;
        let event = client.set_get_eventhandle().map_err(|e| e.to_string())?;
        let capture = client.get_audiocaptureclient().map_err(|e| e.to_string())?;
        client.start_stream().map_err(|e| e.to_string())?;
        Ok((client, capture, event))
    });
    let (client, capture, event) = match opened {
        Ok(stream) => stream,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let _ = ready.send(Ok(()));

    let mut raw: VecDeque<u8> = VecDeque::new();
    while !stop.load(Ordering::SeqCst) {
        if event.wait_for_event(100).is_err() {
            continue; // nothing played this period
        }
        loop {
            match capture.get_next_packet_size() {
                Ok(Some(frames)) if frames > 0 => {
                    if let Err(e) = capture.read_from_device_to_deque(&mut raw) {
                        log::warn!("echo reference: capture of process {root} failed: {e}");
                        return;
                    }
                }
                Ok(_) => break,
                Err(e) => {
                    log::warn!("echo reference: capture of process {root} failed: {e}");
                    return;
                }
            }
        }
        let mut queue = played.lock().unwrap();
        let mut bytes = raw.drain(..raw.len() / 4 * 4);
        while let (Some(a), Some(b), Some(c), Some(d)) = (bytes.next(), bytes.next(), bytes.next(), bytes.next()) {
            queue.push_back(f32::from_le_bytes([a, b, c, d]));
        }
        if queue.len() > MAX_QUEUED {
            let excess = queue.len() - CHUNK_FRAMES * 2;
            queue.drain(..excess);
        }
    }
    let _ = client.stop_stream();
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::level_db;

    /// Plays a 2 s, 440 Hz tone at -26 dBFS in a child PowerShell (out of the speakers) and checks
    /// the reference captured from that process hears it. `JAYA_JAYC_PID=<jay-c.exe pid>` also
    /// prints the WebView2 it would capture. `cargo test -- --ignored echo`
    #[test]
    #[ignore = "plays a tone out of the speakers"]
    fn the_reference_hears_the_process_it_captures() {
        if let Some(pid) = std::env::var("JAYA_JAYC_PID").ok().and_then(|p| p.parse().ok()) {
            println!("Jay-C {pid}: its WebView2 is {:?}", webview_of(pid));
        }
        let wav = std::env::temp_dir().join(format!("jaya-echo-{}.wav", std::process::id()));
        let samples: Vec<i16> = (0..SAMPLE_RATE * 2).map(|i| ((i as f32 * 2.0 * std::f32::consts::PI * 440.0 / SAMPLE_RATE as f32).sin() * 0.07 * 32767.0) as i16).collect();
        let mut bytes = Vec::new();
        let data = (samples.len() * 2) as u32;
        for part in [&b"RIFF"[..], &(36 + data).to_le_bytes(), b"WAVEfmt ", &16u32.to_le_bytes(), &1u16.to_le_bytes(), &1u16.to_le_bytes()] {
            bytes.extend_from_slice(part);
        }
        for part in [SAMPLE_RATE.to_le_bytes(), (SAMPLE_RATE * 2).to_le_bytes()] {
            bytes.extend_from_slice(&part);
        }
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data.to_le_bytes());
        bytes.extend(samples.iter().flat_map(|s| s.to_le_bytes()));
        std::fs::write(&wav, bytes).unwrap();

        let mut child = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &format!("Start-Sleep -Milliseconds 500; (New-Object Media.SoundPlayer '{}').PlaySync()", wav.display())])
            .spawn()
            .unwrap();
        let reference = EchoReference::start(child.id()).unwrap();
        let (mut heard, end) = (Vec::new(), Instant::now() + Duration::from_secs(4));
        while Instant::now() < end {
            heard.extend(reference.played.lock().unwrap().drain(..));
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = child.wait();
        let _ = std::fs::remove_file(&wav);
        let loudest = heard.chunks(SAMPLE_RATE as usize / 10).map(level_db).fold(f32::NEG_INFINITY, f32::max);
        println!("captured {:.1} s, loudest 100 ms: {loudest:.1} dBFS", heard.len() as f32 / SAMPLE_RATE as f32);
        assert!(loudest > -32.0, "the tone (-26 dBFS) reached the reference: {loudest:.1} dBFS");
    }
}
