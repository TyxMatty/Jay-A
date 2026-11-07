//! Installing VB-CABLE (the virtual cable behind "Microphone for other apps", docs/ARQUITETURA.md
//! D4) for the user, from the setup guide (ui/cable-guide.js). VB-CABLE is donationware that may
//! not be shipped inside another installer without VB-Audio's distributor licence, so Jay-A
//! downloads VB-Audio's own package from their site when the user asks, checks it is signed by
//! VB-Audio, and opens VB-Audio's setup; Windows asks the user for administrator rights.
//!
//! Windows' own tools do the work: curl.exe and tar.exe (Windows 10 1803 and later) and PowerShell
//! for the signature and the elevation, so there's no new dependency.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

/// VB-Audio's current Windows package (vb-audio.com/Cable).
const PACK: &str = "https://download.vb-audio.com/Download_CABLE/VBCABLE_Driver_Pack46.zip";
/// Where to get it by hand.
const SITE: &str = "https://vb-audio.com/Cable/";
/// In every one of VB-Audio's code-signing certificates (Vincent Burel's business registration).
const VB_AUDIO: &str = "SERIALNUMBER=423 734 177";
/// No console window flashing up over the app.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn run(program: &str, args: &[&str]) -> Result<std::process::ExitStatus, String> {
    Command::new(program).args(args).creation_flags(CREATE_NO_WINDOW).status().map_err(|e| format!("{program}: {e}"))
}

/// A path inside a PowerShell single-quoted string.
fn quoted(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}

/// Downloads VB-CABLE, checks VB-Audio's signature and runs its setup (the user clicks "Install
/// Driver" there and allows administrator rights). Returns when the setup window closes.
#[tauri::command(async)]
pub fn cable_install() -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("jaya-vbcable-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = install(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn install(dir: &Path) -> Result<(), String> {
    let zip = dir.join("VBCABLE_Driver_Pack.zip");
    let zip_arg = zip.display().to_string();
    if !run("curl.exe", &["--location", "--fail", "--silent", "--show-error", "--max-time", "180", "--output", &zip_arg, PACK])?.success() {
        return Err("couldn't download VB-CABLE from vb-audio.com".into());
    }
    let dir_arg = dir.display().to_string();
    if !run("tar.exe", &["-xf", &zip_arg, "-C", &dir_arg])?.success() {
        return Err("the VB-CABLE package didn't open".into());
    }
    let setup = dir.join(if cfg!(target_arch = "aarch64") { "VBCABLE_Setup_arm64.exe" } else { "VBCABLE_Setup_x64.exe" });
    if !setup.is_file() {
        return Err("the VB-CABLE package has no setup for this PC".into());
    }
    // Only VB-Audio's own, untouched setup runs (a valid signature from their certificate).
    let check = format!(
        "$s = Get-AuthenticodeSignature -LiteralPath {}; if ($s.Status -eq 'Valid' -and $s.SignerCertificate.Subject -like '*{VB_AUDIO}*') {{ exit 0 }} else {{ exit 3 }}",
        quoted(&setup)
    );
    if !run("powershell.exe", &["-NoProfile", "-NonInteractive", "-Command", &check])?.success() {
        return Err("the downloaded setup isn't signed by VB-Audio, so it wasn't opened".into());
    }
    let open = format!("Start-Process -FilePath {} -Verb RunAs -Wait", quoted(&setup));
    if !run("powershell.exe", &["-NoProfile", "-NonInteractive", "-Command", &open])?.success() {
        return Err("the installation was cancelled".into());
    }
    Ok(())
}

/// VB-CABLE's page, in the browser (a fixed address; the app has no other way to open links).
#[tauri::command]
pub fn cable_site() -> Result<(), String> {
    Command::new("explorer.exe").arg(SITE).creation_flags(CREATE_NO_WINDOW).spawn().map(drop).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::quoted;
    use std::path::Path;

    #[test]
    fn paths_stay_inside_their_powershell_quotes() {
        assert_eq!(quoted(Path::new(r"C:\Users\ana\Temp\x.exe")), r"'C:\Users\ana\Temp\x.exe'");
        assert_eq!(quoted(Path::new(r"C:\Users\o'neil\x.exe")), r"'C:\Users\o''neil\x.exe'");
    }
}
