//! Updating Jay-A from its GitHub releases with the line's shared updater (`jay-i`): the installer
//! comes only from this repository's latest release and runs only if its SHA-256 matches the digest
//! GitHub computed (docs/ARQUITETURA.md, section 2, "O setup do Jay-I e a autoatualização"). The page sends nothing: no address, no path.

use crate::audio::MicState;
use crate::convert::ConvertState;
use jay_i::{Mode, Release};
use serde::Serialize;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager};

const REPO: &str = "TyxMatty/Jay-A";
const AGENT: &str = concat!("Jay-A/", env!("CARGO_PKG_VERSION"));
const DOWNLOAD_EVENT: &str = "update-download";

#[derive(Clone, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
enum Event {
    Progress { done: u64, total: u64 },
    Error { code: &'static str, message: String },
}

#[derive(Default)]
pub struct Update {
    /// The answer of the one check made in this process (a page reload doesn't ask GitHub again).
    checked: OnceLock<Option<String>>,
    running: AtomicBool,
}

impl Update {
    /// True for the one caller that may run an update; the others wait for `end`.
    fn begin(&self) -> bool {
        !self.running.swap(true, Ordering::Relaxed)
    }

    fn end(&self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

/// The version to offer: the release is newer than `current` and has an installer that passes
/// every check (this repository's address, a SHA-256 from GitHub).
fn offer(release: &Release, current: &str) -> Option<String> {
    jay_i::pick_installer(REPO, release).ok()?;
    jay_i::newer(&release.tag_name, current).then(|| release.tag_name.trim_start_matches(['v', 'V']).to_string())
}

fn offered(current: &str) -> Option<String> {
    offer(&jay_i::latest(REPO, AGENT).ok()??, current)
}

/// Why an update must wait, as the code the page shows: Jay-C is using the microphone or is in a
/// call (a restart would cut the voice mid-sentence), or a conversion is running (quitting would
/// leave half-written files).
fn refusal(app: &AppHandle) -> Option<&'static str> {
    if app.state::<MicState>().in_call() {
        Some("call")
    } else if app.state::<ConvertState>().running() {
        Some("working")
    } else {
        None
    }
}

/// The newer version on GitHub, or None (up to date, no release, no network: silent).
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Option<String> {
    let current = app.package_info().version.to_string();
    tauri::async_runtime::spawn_blocking(move || app.state::<Update>().checked.get_or_init(|| offered(&current)).clone())
        .await
        .ok()
        .flatten()
}

/// Downloads the installer, runs it and quits so it can replace the app; progress and errors come
/// as `update-download` events. Refused (the error is a code) while an update runs, while Jay-C is
/// using the microphone or in a call, or while a conversion runs.
#[tauri::command]
pub fn update_install(app: AppHandle) -> Result<(), String> {
    let update = app.state::<Update>();
    if !update.begin() {
        return Err("busy".into());
    }
    if let Some(code) = refusal(&app) {
        update.end();
        return Err(code.into());
    }
    std::thread::spawn(move || {
        let current = app.package_info().version.to_string();
        let fail = |code: &'static str, message: String| {
            app.state::<Update>().end();
            let _ = app.emit(DOWNLOAD_EVENT, Event::Error { code, message });
        };
        let release = match jay_i::latest(REPO, AGENT) {
            Ok(Some(release)) if offer(&release, &current).is_some() => release,
            Ok(_) => return fail("none", "no newer release".into()),
            Err(e) => return fail("network", e),
        };
        let installer = match jay_i::pick_installer(REPO, &release) {
            Ok(installer) => installer,
            Err(e) => return fail("install", e),
        };
        let progress = |done, total| {
            let _ = app.emit(DOWNLOAD_EVENT, Event::Progress { done, total });
        };
        let path = match jay_i::download(&installer, AGENT, progress) {
            Ok(path) => path,
            Err(e) => return fail("network", e),
        };
        // A call or a conversion started during the download would be cut by the exit.
        if let Some(code) = refusal(&app) {
            let _ = std::fs::remove_dir_all(path.parent().unwrap_or(&path));
            return fail(code, "Jay-A became busy".into());
        }
        if let Err(e) = jay_i::run(&path, Mode::Update { relaunch: true }).spawn() {
            return fail("install", e.to_string());
        }
        // The tray's Quit path: settings are written on every change, so nothing is left to save.
        app.exit(0);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jay_i::ReleaseAsset;

    const URL: &str = "https://github.com/TyxMatty/Jay-A/releases/download/v0.3.5/Jay-A_0.3.5_x64-setup.exe";
    const DIGEST: &str = "sha256:848005e8a2f42200b36f5553fd144a9d8aefdcbddbbf1ad49b56c65f473d63de";

    fn release(tag: &str, url: &str, digest: Option<&str>) -> Release {
        Release {
            tag_name: tag.into(),
            html_url: String::new(),
            assets: vec![ReleaseAsset { name: "Jay-A_0.3.5_x64-setup.exe".into(), size: 5_000_000, browser_download_url: url.into(), digest: digest.map(Into::into) }],
        }
    }

    #[test]
    fn offers_only_a_newer_version_with_a_checked_installer() {
        assert_eq!(offer(&release("v0.3.5", URL, Some(DIGEST)), "0.3.4"), Some("0.3.5".into()));
        assert_eq!(offer(&release("V0.3.5", URL, Some(DIGEST)), "0.3.4"), Some("0.3.5".into()));
        assert_eq!(offer(&release("v0.3.4", URL, Some(DIGEST)), "0.3.4"), None);
        assert_eq!(offer(&release("v0.3.3", URL, Some(DIGEST)), "0.3.4"), None);
        assert_eq!(offer(&release("nightly", URL, Some(DIGEST)), "0.3.4"), None);
        // Newer, but the installer fails a check: another repository, no digest, no installer.
        assert_eq!(offer(&release("v0.3.5", &URL.replace("Jay-A/", "Jay-E/"), Some(DIGEST)), "0.3.4"), None);
        assert_eq!(offer(&release("v0.3.5", URL, None), "0.3.4"), None);
        let empty = Release { tag_name: "v0.3.5".into(), html_url: String::new(), assets: vec![] };
        assert_eq!(offer(&empty, "0.3.4"), None);
    }

    #[test]
    fn a_second_update_is_refused_until_the_first_ends() {
        let update = Update::default();
        assert!(update.begin());
        assert!(!update.begin());
        update.end();
        assert!(update.begin());
    }
}
