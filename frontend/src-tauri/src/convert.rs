//! Convert (stage 5, docs/ARQUITETURA.md section 9): a batch of files, one at a time per CPU core,
//! each with its own result. Files are passed by path (chosen in Windows' dialog or dropped on the
//! window), never as bytes: they can be hours long. The engine does the work
//! (jay_audio::files::convert); this decides the output names and runs the batch.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use jay_audio::files::convert::{Format, Options, convert};
use serde::Deserialize;
use serde_json::{Value, json};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

/// The extensions the dialog offers (what the engine reads).
const AUDIO: &[&str] = &["wav", "flac", "mp3", "ogg", "opus", "m4a", "aac", "mp4", "aif", "aiff", "caf", "mkv", "webm"];

/// The running batch's cancel flag.
#[derive(Default)]
pub struct ConvertState(Mutex<Option<Arc<AtomicBool>>>);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    files: Vec<String>,
    /// "wav16" | "wav24" | "wav32" | "flac" | "mp3" | "ogg" | "opus"
    format: String,
    /// kbps for MP3 and Opus; quality ×10 (0–10) for OGG.
    #[serde(default)]
    quality: Option<u32>,
    #[serde(default)]
    rate: Option<u32>,
    #[serde(default)]
    channels: Option<usize>,
    #[serde(default)]
    normalize: bool,
    /// None: next to each file.
    #[serde(default)]
    folder: Option<String>,
    /// When the output exists: "replace", "keep" (both: "name (1).ext") or "skip".
    on_exists: String,
}

fn format_of(request: &Request) -> Result<Format, String> {
    format_of_name(&request.format, request.quality)
}

/// "wav16" | "wav24" | "wav32" | "flac" | "mp3" | "ogg" | "opus", with the quality (kbps; OGG: q × 10).
pub fn format_of_name(name: &str, q: Option<u32>) -> Result<Format, String> {
    Ok(match name {
        "wav16" => Format::Wav { bits: 16 },
        "wav24" => Format::Wav { bits: 24 },
        "wav32" => Format::Wav { bits: 32 },
        "flac" => Format::Flac,
        "mp3" => Format::Mp3 { kbps: q.unwrap_or(192) },
        "ogg" => Format::Ogg { quality: q.unwrap_or(5) as f32 / 10.0 },
        "opus" => Format::Opus { kbps: q.unwrap_or(96) },
        other => return Err(format!("unknown format {other}")),
    })
}

/// Where `input` goes; None = skip it. Never the input itself.
fn output_for(input: &Path, folder: Option<&Path>, ext: &str, on_exists: &str) -> Option<PathBuf> {
    let dir = folder.map(Path::to_path_buf).or_else(|| input.parent().map(Path::to_path_buf)).unwrap_or_default();
    let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "audio".into());
    let first = dir.join(format!("{stem}.{ext}"));
    let taken = |p: &Path| p.exists() || p == input;
    if !taken(&first) {
        return Some(first);
    }
    match on_exists {
        "replace" if first != input => Some(first),
        "skip" => None,
        _ => (1..).map(|n| dir.join(format!("{stem} ({n}).{ext}"))).find(|p| !taken(p)),
    }
}

#[tauri::command(async)]
pub fn pick_audio_files(app: AppHandle) -> Vec<String> {
    let picked = app.dialog().file().add_filter("Audio", AUDIO).blocking_pick_files().unwrap_or_default();
    picked.into_iter().map(|p| p.to_string()).collect()
}

#[tauri::command(async)]
pub fn pick_folder(app: AppHandle) -> Option<String> {
    app.dialog().file().blocking_pick_folder().map(|p| p.to_string())
}

/// Runs the batch; `on_event` gets `{ index, state: "running" | "done" | "error" | "skipped" |
/// "cancelled", progress?, output?, error? }` per file and `{ finished: true }` at the end.
#[tauri::command(async)]
pub fn convert_start(state: State<'_, ConvertState>, request: Request, on_event: Channel<Value>) -> Result<(), String> {
    let format = format_of(&request)?;
    let options = Options { format, rate: request.rate, channels: request.channels, normalize: request.normalize };
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut running = state.0.lock().map_err(|e| e.to_string())?;
        if running.as_ref().is_some_and(|c| Arc::strong_count(c) > 1) {
            return Err("a batch is already running".into());
        }
        *running = Some(cancel.clone());
    }
    // The batch ends off this command's thread, so the page gets its answer at once.
    std::thread::spawn(move || {
        run_batch(request, format, options, &cancel, &move |v| drop(on_event.send(v)));
    });
    Ok(())
}

/// Converts every file, one per CPU core at a time; `send` gets each file's events, then
/// `{ finished: true }`.
fn run_batch(request: Request, format: Format, options: Options, cancel: &Arc<AtomicBool>, send: &(dyn Fn(Value) + Sync)) {
    let request = Arc::new(request);
    let next = Arc::new(AtomicUsize::new(0));
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get()).min(request.files.len()).max(1);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let (request, next, cancel) = (request.clone(), next.clone(), cancel.clone());
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(file) = request.files.get(index) else { break };
                    if cancel.load(Ordering::SeqCst) {
                        send(json!({ "index": index, "state": "cancelled" }));
                        continue;
                    }
                    let input = PathBuf::from(file);
                    let folder = request.folder.as_deref().map(Path::new);
                    let Some(output) = output_for(&input, folder, format.extension(), &request.on_exists) else {
                        send(json!({ "index": index, "state": "skipped" }));
                        continue;
                    };
                    send(json!({ "index": index, "state": "running", "progress": 0 }));
                    let mut last = 0.0f32;
                    let mut progress = |p: f32| {
                        if p - last >= 0.02 {
                            last = p;
                            send(json!({ "index": index, "state": "running", "progress": p }));
                        }
                    };
                    match convert(&input, &output, options, &cancel, &mut progress) {
                        Ok(()) => send(json!({ "index": index, "state": "done", "output": output.display().to_string() })),
                        Err(e) if cancel.load(Ordering::SeqCst) => send(json!({ "index": index, "state": "cancelled", "error": e })),
                        Err(e) => send(json!({ "index": index, "state": "error", "error": e })),
                    }
                }
            });
        }
    });
    send(json!({ "finished": true }));
}

#[tauri::command(async)]
pub fn convert_cancel(state: State<'_, ConvertState>) {
    if let Ok(running) = state.0.lock()
        && let Some(cancel) = running.as_ref()
    {
        cancel.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jay_audio::files::decode::Reader;

    /// A 16-bit WAV with a 440 Hz tone.
    fn wav(path: &Path, rate: u32, channels: u16, seconds: f32) {
        let frames = (rate as f32 * seconds) as usize;
        let mut data = Vec::new();
        for i in 0..frames {
            let s = (0.4 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin() * 32767.0) as i16;
            for _ in 0..channels {
                data.extend_from_slice(&s.to_le_bytes());
            }
        }
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        for v in [16u32.to_le_bytes().to_vec(), 1u16.to_le_bytes().to_vec(), channels.to_le_bytes().to_vec(), rate.to_le_bytes().to_vec(), (rate * channels as u32 * 2).to_le_bytes().to_vec(), (channels * 2).to_le_bytes().to_vec(), 16u16.to_le_bytes().to_vec()] {
            bytes.extend(v);
        }
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend(data);
        std::fs::write(path, bytes).unwrap();
    }

    fn seconds(path: &Path) -> f32 {
        let mut r = Reader::open(path).unwrap();
        let mut n = 0;
        while let Some(b) = r.next_block().unwrap() {
            n += b.len();
        }
        n as f32 / r.channels as f32 / r.rate as f32
    }

    fn request(files: &[PathBuf], format: &str, folder: &Path) -> Request {
        let files = files.iter().map(|p| p.display().to_string()).collect();
        Request { files, format: format.into(), quality: None, rate: None, channels: None, normalize: false, folder: Some(folder.display().to_string()), on_exists: "keep".into() }
    }

    /// Stage 5's "done when": 10 mixed files to each format; the broken one fails alone.
    #[test]
    fn a_mixed_batch_where_a_broken_file_fails_alone() {
        let dir = std::env::temp_dir().join(format!("jaya-batch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("in")).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut inputs = Vec::new();
        for (name, rate, ch, secs) in [("a", 44_100, 2, 2.0), ("b", 48_000, 1, 1.0), ("c", 22_050, 1, 1.5), ("d", 96_000, 2, 1.0), ("e", 32_000, 2, 1.0)] {
            let path = dir.join("in").join(format!("{name}.wav"));
            wav(&path, rate, ch, secs);
            inputs.push((path, secs));
        }
        // Two WAVs as they are, and two files in each other format, made by the engine.
        let mut mixed: Vec<(PathBuf, f32)> = inputs[..2].to_vec();
        for (i, format) in [Format::Mp3 { kbps: 128 }, Format::Ogg { quality: 0.4 }, Format::Opus { kbps: 96 }, Format::Flac].into_iter().enumerate() {
            for (input, secs) in &inputs[2 * (i % 2) + 1..2 * (i % 2) + 3] {
                let out = dir.join("in").join(format!("{}-{i}.{}", input.file_stem().unwrap().to_string_lossy(), format.extension()));
                convert(input, &out, Options { format, rate: None, channels: None, normalize: false }, &cancel, &mut |_| {}).unwrap();
                mixed.push((out, *secs));
            }
        }
        assert_eq!(mixed.len(), 10);
        let broken = dir.join("in").join("broken.mp3");
        std::fs::write(&broken, [b"ID3".as_slice(), &[7u8; 4000]].concat()).unwrap();
        let mut files: Vec<PathBuf> = mixed.iter().map(|(p, _)| p.clone()).collect();
        files.push(broken);

        for format in ["wav16", "flac", "mp3", "ogg", "opus"] {
            let folder = dir.join(format);
            std::fs::create_dir_all(&folder).unwrap();
            let request = request(&files, format, &folder);
            let options = Options { format: format_of(&request).unwrap(), rate: None, channels: None, normalize: false };
            let events = Mutex::new(Vec::new());
            let started = std::time::Instant::now();
            run_batch(request, options.format, options, &cancel, &|v| events.lock().unwrap().push(v));
            let events = events.into_inner().unwrap();
            let ends: Vec<&Value> = events.iter().filter(|e| e["state"] != "running" && e.get("index").is_some()).collect();
            let done: Vec<&Value> = ends.iter().copied().filter(|e| e["state"] == "done").collect();
            let failed: Vec<&Value> = ends.iter().copied().filter(|e| e["state"] == "error").collect();
            println!("{format}: {} done, {} failed ({}) in {} ms", done.len(), failed.len(), failed[0]["error"], started.elapsed().as_millis());
            assert_eq!((done.len(), failed.len()), (10, 1), "{format}");
            assert_eq!(failed[0]["index"], 10, "the broken file is the one that failed");
            assert_eq!(events.last().unwrap()["finished"], true);
            for e in done {
                let output = PathBuf::from(e["output"].as_str().unwrap());
                let expected = mixed[e["index"].as_u64().unwrap() as usize].1;
                let got = seconds(&output);
                assert!((got - expected).abs() < 0.08, "{}: {got} s, expected {expected}", output.display());
            }
            assert!(std::fs::read_dir(&folder).unwrap().flatten().all(|f| !f.path().display().to_string().ends_with(".tmp")));
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn output_names_never_overwrite_by_surprise() {
        let dir = std::env::temp_dir().join(format!("jaya-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("song.wav");
        std::fs::write(&input, b"x").unwrap();
        // Same format, same folder: never the input, whatever the choice.
        for choice in ["replace", "keep"] {
            assert_eq!(output_for(&input, None, "wav", choice), Some(dir.join("song (1).wav")));
        }
        assert_eq!(output_for(&input, None, "wav", "skip"), None);
        assert_eq!(output_for(&input, None, "mp3", "skip"), Some(dir.join("song.mp3")));
        std::fs::write(dir.join("song.mp3"), b"x").unwrap();
        assert_eq!(output_for(&input, None, "mp3", "replace"), Some(dir.join("song.mp3")));
        assert_eq!(output_for(&input, None, "mp3", "keep"), Some(dir.join("song (1).mp3")));
        assert_eq!(output_for(&input, Some(Path::new("C:/out")), "ogg", "keep"), Some(Path::new("C:/out").join("song.ogg")));
        std::fs::remove_dir_all(&dir).ok();
    }
}
