//! Jay-A's audio engine (docs/ARQUITETURA.md): 48 kHz f32, cleaned in 10 ms frames, delivered in
//! 20 ms chunks. Stage 1: the microphone through WebRTC's audio processing, its level for the meter,
//! and the monitor ("hear my microphone"); WASAPI, so Windows only. Stage 2: the effect chain
//! (`dsp`), plain Rust on any platform. Stage 3: the Jay Link server (`link`), which hands the
//! cleaned mic to Jay-C, with what Jay-C plays as the echo reference.

pub const SAMPLE_RATE: u32 = 48_000;
/// 20 ms at 48 kHz: Opus frames, and the Jay Link's AUDIO blocks.
pub const CHUNK_FRAMES: usize = 960;

pub mod dsp;

/// Jay-C's file as is (plugins/tauri-plugin-system-audio/src/apm.rs), so fixes copy over unchanged.
#[cfg(windows)]
pub mod apm;
#[cfg(windows)]
mod echo;
#[cfg(windows)]
pub mod link;
#[cfg(windows)]
mod mic;
#[cfg(windows)]
mod monitor;

#[cfg(windows)]
pub use mic::{Mic, MicDevice, MicOptions, list_mics};

/// Schedules the calling thread as audio (Windows MMCSS, "Pro Audio"), ahead of ordinary work. With
/// the CPU busy (a game, a video encode), a normal-priority thread missed its 20 ms: measured with
/// the CPU at 100%, the mic's chunks came up to 83 ms apart, and Jay-C took that as a stalled Jay-A.
pub fn audio_thread_priority() {
    #[cfg(windows)]
    {
        let mut task = 0u32;
        // The registration ends with the thread.
        if let Err(e) = unsafe { windows::Win32::System::Threading::AvSetMmThreadCharacteristicsW(windows::core::w!("Pro Audio"), &mut task) } {
            log::warn!("no audio priority for this thread: {e}");
        }
    }
}

/// RMS level of `samples` in dBFS; -inf for digital silence.
pub fn level_db(samples: &[f32]) -> f32 {
    let sum: f32 = samples.iter().map(|s| s * s).sum();
    10.0 * (sum / samples.len().max(1) as f32).log10()
}

#[cfg(test)]
mod tests {
    use super::level_db;

    #[test]
    fn level_of_known_signals() {
        assert_eq!(level_db(&[0.0; 960]), f32::NEG_INFINITY);
        assert!((level_db(&[1.0; 960]) - 0.0).abs() < 1e-4, "full scale is 0 dBFS");
        assert!((level_db(&[0.5, -0.5].repeat(480)) - -6.0206).abs() < 1e-3, "half scale is -6 dBFS");
        // A full-scale sine is -3 dBFS RMS.
        let sine: Vec<f32> = (0..960).map(|i| (i as f32 * 2.0 * std::f32::consts::PI * 1000.0 / 48_000.0).sin()).collect();
        assert!((level_db(&sine) - -3.0103).abs() < 0.01);
    }
}
