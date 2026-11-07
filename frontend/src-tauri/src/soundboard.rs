//! The soundboard (stage 4, docs/ARQUITETURA.md section 7): the clip library, playing a clip in the
//! call and in the headphones, "stop all", and the global shortcuts.
//!
//! Imported files are copied to `soundboard/` in the app's data folder (checked first: decoded, at
//! most 20 s); the list and the settings are `soundboard` in config.json. A clip is decoded the
//! first time it plays and kept in memory (48 kHz stereo, ~0.4 MB a second).
//! shortcut: every played clip stays decoded in memory; an LRU cache if libraries get past ~200 MB.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use jay_audio::dsp::params::db_to_gain;
use jay_audio::files::decode::decode;
use jay_audio::playback::Playback;
use jay_audio::soundboard::{self, Clip, Player};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::audio::{MicState, config_set, file_id};

/// Longer clips are refused at import (the editor, stage 6, will trim them).
const MAX_SECONDS: f32 = 20.0;
/// Bigger files are refused before decoding (a 20 s WAV at 96 kHz/32 bits stereo is ~15 MB).
const MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClipInfo {
    id: String,
    name: String,
    /// The copy in `soundboard/`.
    file: String,
    /// dB, −30 to +6.
    #[serde(default)]
    volume: f32,
    #[serde(default)]
    hotkey: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Settings {
    clips: Vec<ClipInfo>,
    /// Destinations: Jay-C's call (while it uses the mic) and the headphones.
    call: bool,
    headphones: bool,
    /// dB, −30 to +6, on top of each clip's.
    volume: f32,
    stop_hotkey: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { clips: Vec::new(), call: true, headphones: true, volume: 0.0, stop_hotkey: None }
    }
}

pub struct SoundboardState {
    board: Mutex<Board>,
    dir: PathBuf,
    config: PathBuf,
}

#[derive(Default)]
struct Board {
    settings: Settings,
    decoded: HashMap<String, Clip>,
    headphones: Option<(Player, Playback)>,
    /// Shortcuts Windows refused (taken by another app).
    refused: Vec<String>,
}

impl SoundboardState {
    pub fn load(config_dir: PathBuf, data_dir: PathBuf) -> SoundboardState {
        let config = config_dir.join("config.json");
        let settings = fs::read_to_string(&config)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| serde_json::from_value(v["soundboard"].clone()).ok())
            .unwrap_or_default();
        let board = Board { settings, ..Board::default() };
        SoundboardState { board: Mutex::new(board), dir: data_dir.join("soundboard"), config }
    }

    fn lock(&self) -> Result<MutexGuard<'_, Board>, String> {
        self.board.lock().map_err(|e| e.to_string())
    }

    fn save(&self, board: &Board) -> Result<(), String> {
        config_set(&self.config, "soundboard", json!(board.settings))
    }

    /// Plays clip `id` on the destinations that are on.
    fn play(&self, mic: &MicState, id: &str) -> Result<(), String> {
        let mut board = self.lock()?;
        let info = board.settings.clips.iter().find(|c| c.id == id).cloned().ok_or("no such clip")?;
        let clip = match board.decoded.get(id) {
            Some(clip) => clip.clone(),
            None => {
                let clip: Clip = decode(&self.dir.join(&info.file), MAX_SECONDS)?.into();
                board.decoded.insert(info.id.clone(), clip.clone());
                clip
            }
        };
        let gain = db_to_gain(info.volume + board.settings.volume);
        if board.settings.headphones {
            if !board.headphones.as_ref().is_some_and(|(_, out)| out.running()) {
                let (player, mixer) = soundboard::mixer();
                board.headphones = Some((player, Playback::start(mixer, mic.board_echo())?));
            }
            if let Some((player, _)) = &board.headphones {
                player.play(&clip, gain);
            }
        }
        if board.settings.call
            && let Some(player) = mic.call_player()
        {
            player.play(&clip, gain);
        }
        Ok(())
    }

    pub fn stop_all(&self, mic: &MicState) {
        if let Ok(board) = self.lock()
            && let Some((player, _)) = &board.headphones
        {
            player.stop_all();
        }
        if let Some(player) = mic.call_player() {
            player.stop_all();
        }
    }

    fn state(&self, board: &Board) -> Value {
        let mut state = json!(board.settings);
        state["refused"] = json!(board.refused);
        state
    }
}

/// (Re)registers every clip's shortcut and "stop all"'s; the ones Windows refuses are remembered.
pub fn register_hotkeys(app: &AppHandle) {
    let board_state = app.state::<SoundboardState>();
    let Ok(mut board) = board_state.lock() else { return };
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    board.refused.clear();
    let mut wanted: Vec<(String, Option<String>)> = board.settings.clips.iter().filter_map(|c| Some((c.hotkey.clone()?, Some(c.id.clone())))).collect();
    if let Some(stop) = &board.settings.stop_hotkey {
        wanted.push((stop.clone(), None));
    }
    for (hotkey, clip) in wanted {
        let registered = shortcuts.on_shortcut(hotkey.as_str(), move |app, _, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            let (board, mic) = (app.state::<SoundboardState>(), app.state::<MicState>());
            match &clip {
                Some(id) => drop(board.play(&mic, id)),
                None => board.stop_all(&mic),
            }
        });
        if registered.is_err() {
            board.refused.push(hotkey);
        }
    }
}

#[tauri::command(async)]
pub fn soundboard_state(board: State<'_, SoundboardState>) -> Result<Value, String> {
    let locked = board.lock()?;
    Ok(board.state(&locked))
}

/// The file's bytes as the raw body, its name in the `name` header (URI-encoded). Checked by
/// decoding before it joins the library.
#[tauri::command(async)]
pub fn soundboard_import(board: State<'_, SoundboardState>, request: Request<'_>) -> Result<Value, String> {
    let InvokeBody::Raw(bytes) = request.body() else { return Err("expected the file's bytes".into()) };
    if bytes.len() > MAX_BYTES {
        return Err("this file is too big".into());
    }
    let name = request.headers().get("name").and_then(|v| v.to_str().ok()).unwrap_or("clip");
    let name = percent_decode(name);
    let (stem, ext) = name.rsplit_once('.').unwrap_or((&name, "audio"));
    let ext: String = ext.chars().filter(char::is_ascii_alphanumeric).take(5).collect::<String>().to_lowercase();
    let mut locked = board.lock()?;
    let base = file_id(stem);
    let id = (0..).map(|n| if n == 0 { base.clone() } else { format!("{base}-{n}") }).find(|id| !locked.settings.clips.iter().any(|c| &c.id == id)).expect("a free id");
    let file = format!("{id}.{ext}");
    fs::create_dir_all(&board.dir).map_err(|e| e.to_string())?;
    let path = board.dir.join(&file);
    fs::write(&path, bytes).map_err(|e| e.to_string())?;
    let clip: Clip = match decode(&path, MAX_SECONDS) {
        Ok(samples) => samples.into(),
        Err(e) => {
            let _ = fs::remove_file(&path);
            return Err(e);
        }
    };
    locked.decoded.insert(id.clone(), clip);
    let name = stem.trim().chars().take(64).collect();
    locked.settings.clips.push(ClipInfo { id, name, file, volume: 0.0, hotkey: None });
    board.save(&locked)?;
    Ok(board.state(&locked))
}

/// `patch`: any of `name`, `volume` (dB), `hotkey` (string, or null for none).
#[tauri::command(async)]
pub fn soundboard_update(app: AppHandle, board: State<'_, SoundboardState>, id: String, patch: Value) -> Result<Value, String> {
    {
        let mut locked = board.lock()?;
        let clip = locked.settings.clips.iter_mut().find(|c| c.id == id).ok_or("no such clip")?;
        if let Some(name) = patch["name"].as_str() {
            clip.name = name.trim().chars().take(64).collect();
        }
        if let Some(volume) = patch["volume"].as_f64() {
            clip.volume = (volume as f32).clamp(-30.0, 6.0);
        }
        if let Some(hotkey) = patch.get("hotkey") {
            clip.hotkey = hotkey.as_str().map(str::to_string);
        }
        board.save(&locked)?;
    }
    register_hotkeys(&app);
    let locked = board.lock()?;
    Ok(board.state(&locked))
}

#[tauri::command(async)]
pub fn soundboard_remove(app: AppHandle, board: State<'_, SoundboardState>, id: String) -> Result<Value, String> {
    {
        let mut locked = board.lock()?;
        let index = locked.settings.clips.iter().position(|c| c.id == id).ok_or("no such clip")?;
        let clip = locked.settings.clips.remove(index);
        locked.decoded.remove(&id);
        let _ = fs::remove_file(board.dir.join(&clip.file));
        board.save(&locked)?;
    }
    register_hotkeys(&app);
    let locked = board.lock()?;
    Ok(board.state(&locked))
}

/// `patch`: any of `call`, `headphones` (booleans), `volume` (dB), `stopHotkey` (string or null).
#[tauri::command(async)]
pub fn soundboard_settings(app: AppHandle, board: State<'_, SoundboardState>, patch: Value) -> Result<Value, String> {
    {
        let mut locked = board.lock()?;
        let settings = &mut locked.settings;
        if let Some(on) = patch["call"].as_bool() {
            settings.call = on;
        }
        if let Some(on) = patch["headphones"].as_bool() {
            settings.headphones = on;
        }
        if let Some(volume) = patch["volume"].as_f64() {
            settings.volume = (volume as f32).clamp(-30.0, 6.0);
        }
        if let Some(hotkey) = patch.get("stopHotkey") {
            settings.stop_hotkey = hotkey.as_str().map(str::to_string);
        }
        board.save(&locked)?;
    }
    register_hotkeys(&app);
    let locked = board.lock()?;
    Ok(board.state(&locked))
}

#[tauri::command(async)]
pub fn soundboard_play(board: State<'_, SoundboardState>, mic: State<'_, MicState>, id: String) -> Result<(), String> {
    board.play(&mic, &id)
}

#[tauri::command(async)]
pub fn soundboard_stop(board: State<'_, SoundboardState>, mic: State<'_, MicState>) {
    board.stop_all(&mic);
}

/// `%XX` escapes (from the page's encodeURIComponent) back to text.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = text.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::percent_decode;

    #[test]
    fn names_come_back_from_the_page() {
        assert_eq!(percent_decode("Risada%20alta.mp3"), "Risada alta.mp3");
        assert_eq!(percent_decode("A%C3%A7%C3%A3o.wav"), "Ação.wav");
        assert_eq!(percent_decode("bad%zz%"), "bad%zz%");
    }
}
