//! The Filters tab and the Jay Link: the microphone (stage 1), the effect chain (stage 2c) and
//! Jay-C (stage 3).
//!
//! One mic, open while someone uses it: the page's test or Jay-C (in a call), who share it. It is
//! reopened only when its settings change or Jay-C starts or stops using it (the echo reference
//! comes and goes with Jay-C). Hiding the window to the tray ends the test (main.rs), so the
//! Windows microphone indicator doesn't stay on behind the user's back, unless Jay-C uses the mic.
//!
//! Saved in the app's config folder: the mic's settings in `config.json`, the chain in
//! `chain.json` (after every change), user presets as files in `presets/`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use jay_audio::dsp::chain::{ChainControl, ChainSpec};
use jay_audio::dsp::params::COMMON;
use jay_audio::dsp::{Meters, effect, output_limiter, preset};
use jay_audio::link::{LinkEvent, LinkServer, PIPE};
use jay_audio::{Mic, MicDevice, MicOptions};
use serde_json::{Value, json};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State};

pub struct MicState {
    engine: Mutex<Engine>,
    config: PathBuf,
}

#[derive(Default)]
struct Engine {
    options: MicOptions,
    /// No config.json yet: the first Jay-C to connect may bring the user's settings (`import`).
    fresh: bool,
    mic: Option<Mic>,
    /// What `mic` was opened with: its options and Jay-C's process (the echo reference).
    opened: Option<(MicOptions, Option<u32>)>,
    /// The page's test, where its meters and errors go; none = no test.
    page: Arc<Mutex<Option<Channel<Value>>>>,
    /// "Hear my microphone", during the test only.
    monitor: bool,
    link: Option<LinkServer>,
    jay_c: Option<JayC>,
}

/// The Jay-C connected now.
struct JayC {
    pid: u32,
    version: String,
    wants_mic: bool,
    in_call: bool,
    muted: bool,
}

impl MicState {
    pub fn load(config_dir: &Path) -> MicState {
        let config = config_dir.join("config.json");
        let saved = fs::read_to_string(&config).ok();
        let options = saved
            .as_deref()
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .and_then(|v| serde_json::from_value(v["mic"].clone()).ok())
            .unwrap_or_default();
        MicState { engine: Mutex::new(Engine { options, fresh: saved.is_none(), ..Engine::default() }), config }
    }

    fn lock(&self) -> Result<MutexGuard<'_, Engine>, String> {
        self.engine.lock().map_err(|e| e.to_string())
    }

    fn save(&self, engine: &mut Engine, options: MicOptions) -> Result<(), String> {
        write(&self.config, &serde_json::to_string_pretty(&json!({ "mic": options })).map_err(|e| e.to_string())?)?;
        engine.options = options;
        engine.fresh = false;
        Ok(())
    }

    /// The window went to the tray: the test ends (Jay-C keeps the mic if it uses it). True if a
    /// test was running.
    pub fn stop_test(&self, chain: &ChainState) -> bool {
        let Ok(mut engine) = self.lock() else { return false };
        let was = engine.page.lock().unwrap().take().is_some();
        engine.monitor = false;
        let _ = engine.sync(chain);
        was
    }
}

impl Engine {
    /// Opens, reopens or closes the mic for whoever uses it now.
    fn sync(&mut self, chain: &ChainState) -> Result<(), String> {
        let testing = self.page.lock().unwrap().is_some();
        let jay_c = self.jay_c.as_ref().filter(|j| j.wants_mic).map(|j| j.pid);
        let wanted = (testing || jay_c.is_some()).then(|| (self.options.clone(), jay_c));
        if self.opened != wanted {
            self.mic = None;
            self.opened = None;
            if let Some((options, echo_of)) = wanted.clone() {
                let mic = self.open(options, echo_of, chain)?;
                if let Some(link) = &self.link {
                    // Jay-C's reserve follows the mic we use.
                    link.set_state(json!({ "type": "state", "device": mic.device, "deviceId": mic.device_id, "preset": null }));
                }
                self.mic = Some(mic);
                self.opened = wanted;
            }
        }
        match &self.mic {
            Some(mic) => mic.set_monitor(testing && self.monitor),
            None => Ok(()),
        }
    }

    fn open(&self, options: MicOptions, echo_of: Option<u32>, chain: &ChainState) -> Result<Mic, String> {
        let runner = chain.lock()?.runner();
        // The audio thread hands its meters over without allocating; this thread makes the JSON for
        // the page's test, keeping the loudest values between two sends. It ends with the mic.
        let (meters_tx, meters_rx) = sync_channel::<Meters>(8);
        let page = self.page.clone();
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
                        if let Some(page) = page.lock().unwrap().as_ref() {
                            // JSON has no -inf: digital silence is sent as -120 dB.
                            let effects: Vec<[f32; 2]> = l.effects[..l.count].iter().map(|&(i, o)| [i, o]).collect();
                            let _ = page.send(json!({ "level": l.level.max(-120.0), "effects": effects }));
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;

        let link = self.link.as_ref().map(LinkServer::audio);
        let page = self.page.clone();
        // shortcut: a mic that fails mid-call only tells the page; Jay-C notices the missing AUDIO
        // and stays on its reserve until it reconnects. Reopen here if that turns out to matter.
        Mic::start(
            options,
            echo_of,
            runner,
            move |chunk, m| {
                if let Some(link) = &link {
                    link.push(chunk); // dropped unless Jay-C asked for the mic
                }
                let _ = meters_tx.try_send(*m); // full: the page just misses one
            },
            move |error| {
                if let Some(page) = page.lock().unwrap().as_ref() {
                    let _ = page.send(json!({ "error": error }));
                }
            },
        )
    }

    /// What the page shows about Jay-C.
    fn link_state(&self) -> Value {
        match &self.jay_c {
            None => json!({ "connected": false }),
            Some(j) => json!({
                "connected": true,
                "version": j.version,
                "usingMic": j.wants_mic,
                "inCall": j.in_call,
                "muted": j.muted,
                "echoCancellation": j.wants_mic && self.mic.as_ref().is_some_and(|m| m.echo_cancellation),
            }),
        }
    }
}

/// Jay-C's settings from its HELLO (`{ micDevice, micProcessing: { noiseSuppression,
/// autoGainControl }, ... }`). Its gain and gate stay out: the chain has its own.
fn import(options: &mut MicOptions, from: &Value) {
    if let Some(device) = from["micDevice"].as_str() {
        options.device = Some(device.to_string());
    }
    let processing = &from["micProcessing"];
    if let Some(on) = processing["noiseSuppression"].as_bool() {
        options.noise_suppression = on;
    }
    if let Some(on) = processing["autoGainControl"].as_bool() {
        options.auto_gain_control = on;
    }
}

/// Starts the Jay Link server; without it (the pipe is taken), Jay-A runs alone.
pub fn start_link(app: &AppHandle) {
    let handle = app.clone();
    let version = app.package_info().version.to_string();
    match LinkServer::start(PIPE, &version, move |event| on_link(&handle, event)) {
        Ok(server) => {
            if let Ok(mut engine) = app.state::<MicState>().lock() {
                engine.link = Some(server);
            }
        }
        Err(e) => eprintln!("Jay Link: {e}"),
    }
}

fn on_link(app: &AppHandle, event: LinkEvent) {
    let (mic, chain) = (app.state::<MicState>(), app.state::<ChainState>());
    let Ok(mut engine) = mic.lock() else { return };
    match event {
        LinkEvent::Connected { pid, version, import: from } => {
            if engine.fresh
                && let Some(from) = from
            {
                let mut options = engine.options.clone();
                import(&mut options, &from);
                if mic.save(&mut engine, options.clone()).is_ok() {
                    let _ = app.emit("mic-options", options);
                }
            }
            engine.jay_c = Some(JayC { pid, version, wants_mic: false, in_call: false, muted: false });
        }
        LinkEvent::StartMic | LinkEvent::StopMic => {
            if let Some(j) = engine.jay_c.as_mut() {
                j.wants_mic = event == LinkEvent::StartMic;
            }
        }
        LinkEvent::CallState { in_call, muted } => {
            if let Some(j) = engine.jay_c.as_mut() {
                (j.in_call, j.muted) = (in_call, muted);
            }
        }
        LinkEvent::Show => crate::show_main(app),
        LinkEvent::Disconnected => engine.jay_c = None,
    }
    if let Err(error) = engine.sync(&chain)
        && let Some(link) = &engine.link
    {
        link.send(json!({ "type": "error", "message": error }));
    }
    let _ = app.emit("link-state", engine.link_state());
}

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

/// The mic's settings (config.json).
#[tauri::command(async)]
pub fn mic_options(mic: State<'_, MicState>) -> Result<MicOptions, String> {
    Ok(mic.lock()?.options.clone())
}

/// Saves the mic's settings and reopens it with them if it's open (the test, or Jay-C's call).
#[tauri::command(async)]
pub fn set_mic_options(mic: State<'_, MicState>, chain: State<'_, ChainState>, options: MicOptions) -> Result<(), String> {
    let mut engine = mic.lock()?;
    mic.save(&mut engine, options)?;
    engine.sync(&chain)
}

/// The page's test: opens the mic (or joins Jay-C on it) and returns its name. `on_event` gets
/// `{ level, effects: [[in, out], ...] }` (dBFS RMS; linear peaks) about 20 times a second, and
/// `{ error }` if the mic fails later.
#[tauri::command(async)]
pub fn start_mic(mic: State<'_, MicState>, chain: State<'_, ChainState>, monitor: bool, on_event: Channel<Value>) -> Result<String, String> {
    let mut engine = mic.lock()?;
    *engine.page.lock().unwrap() = Some(on_event);
    engine.monitor = monitor;
    if let Err(error) = engine.sync(&chain) {
        engine.page.lock().unwrap().take();
        let _ = engine.sync(&chain);
        return Err(error);
    }
    Ok(engine.mic.as_ref().map(|m| m.device.clone()).unwrap_or_default())
}

#[tauri::command(async)]
pub fn stop_mic(mic: State<'_, MicState>, chain: State<'_, ChainState>) -> Result<(), String> {
    mic.stop_test(&chain);
    Ok(())
}

#[tauri::command(async)]
pub fn set_monitor(mic: State<'_, MicState>, chain: State<'_, ChainState>, on: bool) -> Result<(), String> {
    let mut engine = mic.lock()?;
    engine.monitor = on;
    engine.sync(&chain)
}

/// `{ connected, version?, usingMic?, inCall?, muted?, echoCancellation? }`; the `link-state`
/// event brings the changes.
#[tauri::command(async)]
pub fn link_state(mic: State<'_, MicState>) -> Result<Value, String> {
    Ok(mic.lock()?.link_state())
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
    use super::{MicOptions, file_id, import};
    use serde_json::json;

    #[test]
    fn jay_c_settings_are_imported() {
        // What Jay-C's call.js sends as `jayAImport`.
        let from = json!({ "micDevice": "{0.0.1.00000000}.{abc}", "micGain": 1.5, "micGate": -50,
            "micProcessing": { "echoCancellation": true, "noiseSuppression": false, "autoGainControl": true } });
        let mut options = MicOptions::default();
        import(&mut options, &from);
        assert_eq!(options, MicOptions { device: Some("{0.0.1.00000000}.{abc}".into()), noise_suppression: false, auto_gain_control: true });
        // The Windows default and missing fields keep what's there.
        let mut options = MicOptions { device: None, noise_suppression: false, auto_gain_control: false };
        import(&mut options, &json!({ "micDevice": null }));
        assert_eq!(options, MicOptions { device: None, noise_suppression: false, auto_gain_control: false });
    }

    #[test]
    fn preset_file_names_are_safe() {
        assert_eq!(file_id("Minha Voz!"), "minha-voz");
        assert_eq!(file_id("../../windows/system32"), "windows-system32");
        assert_eq!(file_id("  "), "preset");
        assert_eq!(file_id("Rádio  FM"), "r-dio-fm");
        assert_eq!(file_id(&"a".repeat(100)).len(), 48);
    }
}
