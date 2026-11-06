//! Jay-A's audio engine (docs/ARQUITETURA.md): 48 kHz f32, cleaned in 10 ms frames, delivered in
//! 20 ms chunks. Stage 1: the microphone through WebRTC's audio processing, its level for the meter,
//! and the monitor ("hear my microphone"). Windows only (WASAPI).

pub const SAMPLE_RATE: u32 = 48_000;
/// 20 ms at 48 kHz: Opus frames, and the Jay Link's AUDIO blocks.
pub const CHUNK_FRAMES: usize = 960;

/// Jay-C's file as is (plugins/tauri-plugin-system-audio/src/apm.rs), so fixes copy over unchanged.
#[cfg(windows)]
pub mod apm;
#[cfg(windows)]
mod mic;
#[cfg(windows)]
mod monitor;

#[cfg(windows)]
pub use mic::{Mic, MicDevice, MicOptions, list_mics};

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
