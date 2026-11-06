//! The Filters tab's microphone test (stage 1): the cleaned mic's level for the meter, and the
//! monitor. One mic at a time; hiding the window to the tray closes it (main.rs), so the Windows
//! microphone indicator doesn't stay on behind the user's back.

use std::sync::Mutex;

use jay_audio::{Mic, MicDevice, MicOptions, level_db};
use serde_json::{Value, json};
use tauri::State;
use tauri::ipc::Channel;

#[derive(Default)]
pub struct MicState(pub Mutex<Option<Mic>>);

#[tauri::command]
pub async fn list_mics() -> Result<Vec<MicDevice>, String> {
    jay_audio::list_mics()
}

/// Opens the mic (replacing a running one, e.g. with new options) and returns its name.
/// `on_event` gets `{ level }` every 20 ms (dBFS) and `{ error }` if the mic fails later.
#[tauri::command(async)]
pub fn start_mic(state: State<'_, MicState>, options: MicOptions, monitor: bool, on_event: Channel<Value>) -> Result<String, String> {
    let mut current = state.0.lock().map_err(|e| e.to_string())?;
    current.take();
    let errors = on_event.clone();
    let mic = Mic::start(
        options,
        // JSON has no -inf: digital silence is sent as -120 dB.
        move |chunk| drop(on_event.send(json!({ "level": level_db(chunk).max(-120.0) }))),
        move |error| drop(errors.send(json!({ "error": error }))),
    )?;
    mic.set_monitor(monitor)?;
    let device = mic.device.clone();
    *current = Some(mic);
    Ok(device)
}

#[tauri::command(async)]
pub fn stop_mic(state: State<'_, MicState>) -> Result<(), String> {
    state.0.lock().map_err(|e| e.to_string())?.take();
    Ok(())
}

#[tauri::command(async)]
pub fn set_monitor(state: State<'_, MicState>, on: bool) -> Result<(), String> {
    match state.0.lock().map_err(|e| e.to_string())?.as_ref() {
        Some(mic) => mic.set_monitor(on),
        None => Ok(()),
    }
}
