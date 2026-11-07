//! The audio editor (stage 6, docs/ARQUITETURA.md section 8): one file open at a time, its edits as
//! a list (jay_audio::files::edit), a preview in the headphones and an export through the converter.
//! The original file is never written.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, MutexGuard};

use jay_audio::files::convert::{Options, convert};
use jay_audio::files::edit::{Audio, Edit, peaks, render, to_clip, write_wav};
use jay_audio::playback::Playback;
use jay_audio::soundboard::{self, Clip, Player};
use serde_json::{Value, json};
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::audio::MicState;
use crate::convert::format_of_name;

#[derive(Default)]
pub struct EditorState(Mutex<Editor>);

#[derive(Default)]
struct Editor {
    /// The file's name and its audio as read.
    original: Option<(String, Audio)>,
    edits: Vec<Edit>,
    /// The original with the edits applied.
    rendered: Option<Audio>,
    preview: Option<(Player, Playback)>,
}

impl EditorState {
    fn lock(&self) -> Result<MutexGuard<'_, Editor>, String> {
        self.0.lock().map_err(|e| e.to_string())
    }
}

impl Editor {
    fn rerender(&mut self) {
        self.rendered = self.original.as_ref().map(|(_, audio)| render(audio, &self.edits));
    }

    /// `{ name, rate, channels, frames, edits, peaks: [[min, max], ...] }`, or null with no file.
    fn view(&self, columns: usize) -> Value {
        let (Some((name, _)), Some(audio)) = (&self.original, &self.rendered) else { return Value::Null };
        let peaks: Vec<[f32; 2]> = peaks(audio, columns).into_iter().map(|(lo, hi)| [lo, hi]).collect();
        json!({ "name": name, "rate": audio.rate, "channels": audio.channels, "frames": audio.frames(), "edits": self.edits.len(), "peaks": peaks })
    }
}

#[tauri::command(async)]
pub fn editor_open(editor: State<'_, EditorState>, path: String, columns: usize) -> Result<Value, String> {
    let audio = Audio::read(Path::new(&path))?;
    let name = Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut e = editor.lock()?;
    e.original = Some((name, audio));
    e.edits.clear();
    e.rerender();
    Ok(e.view(columns))
}

#[tauri::command(async)]
pub fn editor_view(editor: State<'_, EditorState>, columns: usize) -> Result<Value, String> {
    Ok(editor.lock()?.view(columns))
}

#[tauri::command(async)]
pub fn editor_edit(editor: State<'_, EditorState>, edit: Edit, columns: usize) -> Result<Value, String> {
    let mut e = editor.lock()?;
    if e.original.is_none() {
        return Err("no file open".into());
    }
    e.edits.push(edit);
    e.rerender();
    Ok(e.view(columns))
}

#[tauri::command(async)]
pub fn editor_undo(editor: State<'_, EditorState>, columns: usize) -> Result<Value, String> {
    let mut e = editor.lock()?;
    e.edits.pop();
    e.rerender();
    Ok(e.view(columns))
}

/// Plays frames `from..to` of the edited audio (all of it when not given) on the default output.
#[tauri::command(async)]
pub fn editor_play(editor: State<'_, EditorState>, mic: State<'_, MicState>, from: Option<usize>, to: Option<usize>) -> Result<(), String> {
    let mut e = editor.lock()?;
    let audio = e.rendered.as_ref().ok_or("no file open")?;
    let to = to.unwrap_or(usize::MAX).min(audio.frames());
    let from = from.unwrap_or(0).min(to);
    let part = Audio { rate: audio.rate, channels: audio.channels, samples: audio.samples[from * audio.channels..to * audio.channels].to_vec() };
    let clip: Clip = to_clip(&part)?.into();
    if let Some((player, _)) = &e.preview {
        player.stop_all();
    }
    if !e.preview.as_ref().is_some_and(|(_, out)| out.running()) {
        let (player, mixer) = soundboard::mixer();
        e.preview = Some((player, Playback::start(mixer, mic.board_echo())?));
    }
    if let Some((player, _)) = &e.preview {
        player.play(&clip, 1.0);
    }
    Ok(())
}

#[tauri::command(async)]
pub fn editor_stop(editor: State<'_, EditorState>) -> Result<(), String> {
    if let Some((player, _)) = &editor.lock()?.preview {
        player.stop_all();
    }
    Ok(())
}

/// Asks where to save, then writes the edited audio there in `format` ("wav16", "mp3"...). Returns
/// the path, or null if the dialog was closed.
#[tauri::command(async)]
pub fn editor_export(app: AppHandle, editor: State<'_, EditorState>, format: String, quality: Option<u32>) -> Result<Option<String>, String> {
    let format = format_of_name(&format, quality)?;
    let (name, audio) = {
        let e = editor.lock()?;
        let name = e.original.as_ref().ok_or("no file open")?.0.clone();
        (name, e.rendered.clone().ok_or("no file open")?)
    };
    let stem = Path::new(&name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "audio".into());
    let ext = format.extension();
    let Some(target) = app.dialog().file().set_file_name(format!("{stem} (edited).{ext}")).add_filter(ext.to_uppercase(), &[ext]).blocking_save_file() else {
        return Ok(None);
    };
    let target = target.into_path().map_err(|e| e.to_string())?;
    let tmp = std::env::temp_dir().join(format!("jaya-editor-{}.wav", std::process::id()));
    write_wav(&audio, &tmp)?;
    let result = convert(&tmp, &target, Options { format, rate: None, channels: None, normalize: false }, &AtomicBool::new(false), &mut |_| {});
    let _ = std::fs::remove_file(&tmp);
    result.map(|()| Some(PathBuf::from(&target).display().to_string()))
}
