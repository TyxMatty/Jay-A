//! Auto-update (docs/ARQUITETURA.md, section 12): at start and every 6 hours, Jay-A looks for a
//! newer release on GitHub (its `latest.json`, signed with the project's key), downloads it in the
//! background and installs it as soon as the microphone is free. Installing closes Jay-A for a few
//! seconds, so never during Jay-C's call or a test; the installer opens it again with the same
//! arguments (`--background` included).
//!
//! The page shows where it stands (`update_state`, and the `update-state` event):
//! `{ state: "checking" | "latest" | "downloading" | "ready" | "error", version?, error? }`, or
//! null when the updater is off (debug builds without `JAYA_UPDATE_URL`).

use std::sync::Mutex;
use std::time::Duration;

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Updater, UpdaterExt};

use crate::audio::MicState;

const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// While someone uses the mic, how often to look again.
const WAIT_FREE: Duration = Duration::from_secs(15);

#[derive(Default)]
pub struct UpdateState(Mutex<Value>);

pub fn start(app: &AppHandle) {
    app.manage(UpdateState::default());
    // Debug builds (npm run dev) only update when told where from, to test it.
    if cfg!(debug_assertions) && std::env::var_os("JAYA_UPDATE_URL").is_none() {
        return;
    }
    let app = app.clone();
    let _ = std::thread::Builder::new().name("jay-a-updater".into()).spawn(move || {
        loop {
            if let Err(error) = tauri::async_runtime::block_on(update(&app)) {
                report(&app, json!({ "state": "error", "error": error }));
            }
            std::thread::sleep(EVERY);
        }
    });
}

async fn update(app: &AppHandle) -> Result<(), String> {
    report(app, json!({ "state": "checking" }));
    let Some(update) = updater(app)?.check().await.map_err(|e| e.to_string())? else {
        report(app, json!({ "state": "latest" }));
        return Ok(());
    };
    let version = update.version.clone();
    report(app, json!({ "state": "downloading", "version": version }));
    // Checked against the public key in tauri.conf.json before anything runs.
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    report(app, json!({ "state": "ready", "version": version }));
    while app.state::<MicState>().in_use() {
        std::thread::sleep(WAIT_FREE); // this thread is the updater's own
    }
    update.install(bytes).map_err(|e| e.to_string()) // Windows: starts the installer and exits
}

/// The configured updater; debug builds take `JAYA_UPDATE_URL` instead (e.g. a local server's
/// latest.json), to test an update without publishing a release.
fn updater(app: &AppHandle) -> Result<Updater, String> {
    #[allow(unused_mut)]
    let mut builder = app.updater_builder();
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("JAYA_UPDATE_URL") {
        let url = url.parse().map_err(|e| format!("JAYA_UPDATE_URL: {e}"))?;
        builder = builder.endpoints(vec![url]).map_err(|e| e.to_string())?;
    }
    builder.build().map_err(|e| e.to_string())
}

fn report(app: &AppHandle, state: Value) {
    *app.state::<UpdateState>().0.lock().unwrap() = state.clone();
    let _ = app.emit("update-state", state);
}

#[tauri::command]
pub fn update_state(state: State<'_, UpdateState>) -> Value {
    state.0.lock().unwrap().clone()
}
