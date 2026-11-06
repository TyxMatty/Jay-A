//! The Filters tab: the microphone test (stage 1) and the effect chain (stage 2c).
//!
//! One mic at a time; hiding the window to the tray closes it (main.rs), so the Windows microphone
//! indicator doesn't stay on behind the user's back. The chain lives here between mic sessions and
//! is saved to `chain.json` in the app's config folder after every change; user presets are files
//! in `presets/` next to it. (Stage 3 moves the rest of the settings there: config.json.)

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::sync_channel;
use std::time::{Duration, Instant};

use jay_audio::dsp::chain::{ChainControl, ChainSpec};
use jay_audio::dsp::params::COMMON;
use jay_audio::dsp::{Meters, effect, output_limiter, preset};
use jay_audio::{Mic, MicDevice, MicOptions};
use serde_json::{Value, json};
use tauri::State;
use tauri::ipc::Channel;

#[derive(Default)]
pub struct MicState(pub Mutex<Option<Mic>>);

pub struct ChainState {
    control: Mutex<ChainControl>,
    file: PathBuf,
    presets: PathBuf,
}

/// The page sees the meters at most this often.
const METERS_EVERY: Duration = Duration::from_millis(50);
/// Bigger preset files are refused (they come from outside).
const MAX_PRESET_BYTES: usize = 256 * 1024;

impl ChainState {
    /// The chain saved last time, or an empty one.
    pub fn load(config_dir: &Path) -> ChainState {
        let file = config_dir.join("chain.json");
        let spec = fs::read_to_string(&file).ok().and_then(|text| preset::parse(&text).ok()).map(|(p, _)| p.chain).unwrap_or_default();
        let control = ChainControl::new(spec).unwrap_or_else(|_| ChainControl::new(ChainSpec::default()).expect("an empty chain is valid"));
        ChainState { control: Mutex::new(control), file, presets: config_dir.join("presets") }
    }

    fn save(&self, control: &ChainControl) -> Result<(), String> {
        let current = preset::Preset { name: String::new(), chain: control.spec().clone(), noise_suppression: None, auto_gain_control: None };
        write(&self.file, &preset::to_json(&current))
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ChainControl>, String> {
        self.control.lock().map_err(|e| e.to_string())
    }
}

/// Writes through a temporary file, so a crash mid-write never leaves half a file.
fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, text).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// What the page draws the chain from: each effect's values (in the order of `COMMON` then its
/// own params) and the latency in samples, the fixed output limiter included.
fn state(control: &ChainControl) -> Value {
    let spec = control.spec();
    json!({
        "slots": spec.slots.iter().map(|s| json!({ "id": s.id, "kind": s.info.kind, "values": s.values })).collect::<Vec<_>>(),
        "latency": spec.latency() + output_limiter().latency(),
    })
}

#[tauri::command]
pub async fn list_mics() -> Result<Vec<MicDevice>, String> {
    jay_audio::list_mics()
}

/// Opens the mic with the chain on it (replacing a running one, e.g. with new options) and returns
/// its name. `on_event` gets `{ level, effects: [[in, out], ...] }` (dBFS RMS; linear peaks)
/// about 20 times a second, and `{ error }` if the mic fails later.
#[tauri::command(async)]
pub fn start_mic(
    mic: State<'_, MicState>,
    chain: State<'_, ChainState>,
    options: MicOptions,
    monitor: bool,
    on_event: Channel<Value>,
) -> Result<String, String> {
    let mut current = mic.0.lock().map_err(|e| e.to_string())?;
    current.take();
    let runner = chain.lock()?.runner();

    // The audio thread hands its meters over without allocating; this thread makes the JSON,
    // keeping the loudest values between two sends. It ends with the mic.
    let (meters_tx, meters_rx) = sync_channel::<Meters>(8);
    let page = on_event.clone();
    std::thread::Builder::new()
        .name("jay-a-meters".into())
        .spawn(move || {
            let mut loudest: Option<Meters> = None;
            let mut sent = Instant::now();
            while let Ok(m) = meters_rx.recv() {
                let l = loudest.get_or_insert(m);
                l.level = l.level.max(m.level);
                l.count = m.count;
                for (a, b) in l.effects.iter_mut().zip(m.effects).take(m.count) {
                    *a = (a.0.max(b.0), a.1.max(b.1));
                }
                if sent.elapsed() >= METERS_EVERY {
                    sent = Instant::now();
                    let l = loudest.take().expect("just set");
                    // JSON has no -inf: digital silence is sent as -120 dB.
                    let effects: Vec<[f32; 2]> = l.effects[..l.count].iter().map(|&(i, o)| [i, o]).collect();
                    let _ = page.send(json!({ "level": l.level.max(-120.0), "effects": effects }));
                }
            }
        })
        .map_err(|e| e.to_string())?;

    let errors = on_event;
    let mic = Mic::start(
        options,
        runner,
        move |m| {
            let _ = meters_tx.try_send(*m); // full: the page just misses one
        },
        move |error| drop(errors.send(json!({ "error": error }))),
    )?;
    mic.set_monitor(monitor)?;
    let device = mic.device.clone();
    *current = Some(mic);
    Ok(device)
}

#[tauri::command(async)]
pub fn stop_mic(mic: State<'_, MicState>) -> Result<(), String> {
    mic.0.lock().map_err(|e| e.to_string())?.take();
    Ok(())
}

#[tauri::command(async)]
pub fn set_monitor(mic: State<'_, MicState>, on: bool) -> Result<(), String> {
    match mic.0.lock().map_err(|e| e.to_string())?.as_ref() {
        Some(mic) => mic.set_monitor(on),
        None => Ok(()),
    }
}

/// The controls every effect has and each effect's own, for the page to build its controls.
#[tauri::command]
pub fn effects_catalog() -> Value {
    json!({
        "common": COMMON,
        "effects": effect::all().iter().map(|e| json!({ "kind": e.kind, "params": e.params })).collect::<Vec<_>>(),
    })
}

#[tauri::command(async)]
pub fn chain_state(chain: State<'_, ChainState>) -> Result<Value, String> {
    Ok(state(&*chain.lock()?))
}

/// `op`: "add" (`kind`), "remove", "duplicate" or "move" (`id`, and `to` for move).
#[tauri::command(async)]
pub fn chain_edit(chain: State<'_, ChainState>, op: String, id: Option<String>, kind: Option<String>, to: Option<usize>) -> Result<Value, String> {
    let mut control = chain.lock()?;
    let mut spec = control.spec().clone();
    let id = id.unwrap_or_default();
    match op.as_str() {
        "add" => drop(spec.add(&kind.unwrap_or_default())?),
        "remove" => spec.remove(&id)?,
        "duplicate" => drop(spec.duplicate(&id)?),
        "move" => spec.move_to(&id, to.unwrap_or(0))?,
        _ => return Err(format!("unknown chain operation {op}")),
    }
    control.set_chain(spec)?;
    chain.save(&control)?;
    Ok(state(&control))
}

/// Sets a parameter (clamped) and returns the value set. `persist`: also save the chain (the page
/// asks for it when a slider is let go, not on every move).
#[tauri::command(async)]
pub fn chain_set_param(chain: State<'_, ChainState>, id: String, key: String, value: f32, persist: bool) -> Result<f32, String> {
    let mut control = chain.lock()?;
    let value = control.set_param(&id, &key, value)?;
    if persist {
        chain.save(&control)?;
    }
    Ok(value)
}

/// Replaces the chain with a preset's. Returns the new state, what was skipped or fixed, and the
/// preset's WebRTC settings (for the page to apply).
#[tauri::command(async)]
pub fn chain_load(chain: State<'_, ChainState>, json: String) -> Result<Value, String> {
    if json.len() > MAX_PRESET_BYTES {
        return Err("this file is too big to be a preset".into());
    }
    let (preset, warnings) = preset::parse(&json)?;
    let mut control = chain.lock()?;
    control.set_chain(preset.chain)?;
    chain.save(&control)?;
    Ok(json!({
        "state": state(&control),
        "warnings": warnings,
        "noiseSuppression": preset.noise_suppression,
        "autoGainControl": preset.auto_gain_control,
    }))
}

/// The current chain as a preset file, with the page's WebRTC settings.
#[tauri::command(async)]
pub fn chain_export(chain: State<'_, ChainState>, name: String, noise_suppression: bool, auto_gain_control: bool) -> Result<String, String> {
    let control = chain.lock()?;
    let preset = preset::Preset {
        name: name.chars().take(64).collect(),
        chain: control.spec().clone(),
        noise_suppression: Some(noise_suppression),
        auto_gain_control: Some(auto_gain_control),
    };
    Ok(preset::to_json(&preset))
}

/// The app's presets and the user's: `[{ id, name, builtIn, json }]`.
#[tauri::command(async)]
pub fn presets_list(chain: State<'_, ChainState>) -> Result<Value, String> {
    let mut list: Vec<Value> = preset::BUILT_IN
        .iter()
        .map(|(id, text)| {
            let name = preset::parse(text).map(|(p, _)| p.name).unwrap_or_default();
            json!({ "id": id, "name": name, "builtIn": true, "json": text })
        })
        .collect();
    let mut user: Vec<Value> = fs::read_dir(&chain.presets)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| {
            let text = fs::read_to_string(e.path()).ok().filter(|t| t.len() <= MAX_PRESET_BYTES)?;
            let (preset, _) = preset::parse(&text).ok()?;
            let id = e.path().file_stem()?.to_string_lossy().into_owned();
            Some(json!({ "id": id, "name": preset.name, "builtIn": false, "json": text }))
        })
        .collect();
    user.sort_by_key(|p| p["name"].as_str().unwrap_or_default().to_lowercase());
    list.extend(user);
    Ok(Value::Array(list))
}

/// Saves `json` (a preset: the current chain from chain_export, or an imported file, checked
/// first) as a user preset named `name`; the same name overwrites. Returns its id.
#[tauri::command(async)]
pub fn preset_save(chain: State<'_, ChainState>, name: String, json: String) -> Result<String, String> {
    if json.len() > MAX_PRESET_BYTES {
        return Err("this file is too big to be a preset".into());
    }
    let (mut preset, _) = preset::parse(&json)?;
    preset.name = name.trim().chars().take(64).collect();
    let id = file_id(&preset.name);
    write(&chain.presets.join(format!("{id}.json")), &preset::to_json(&preset))?;
    Ok(id)
}

#[tauri::command(async)]
pub fn preset_delete(chain: State<'_, ChainState>, id: String) -> Result<(), String> {
    if id != file_id(&id) {
        return Err("invalid preset id".into()); // never a path outside the presets folder
    }
    fs::remove_file(chain.presets.join(format!("{id}.json"))).map_err(|e| e.to_string())
}

/// A preset's file name from its name: lowercase letters, digits and dashes only.
fn file_id(name: &str) -> String {
    let mut id = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            id.push(c);
        } else if !id.ends_with('-') && !id.is_empty() {
            id.push('-');
        }
    }
    let id: String = id.trim_end_matches('-').chars().take(48).collect();
    if id.is_empty() { "preset".into() } else { id }
}

#[cfg(test)]
mod tests {
    use super::file_id;

    #[test]
    fn preset_file_names_are_safe() {
        assert_eq!(file_id("Minha Voz!"), "minha-voz");
        assert_eq!(file_id("../../windows/system32"), "windows-system32");
        assert_eq!(file_id("  "), "preset");
        assert_eq!(file_id("Rádio  FM"), "r-dio-fm");
        assert_eq!(file_id(&"a".repeat(100)).len(), 48);
    }
}
