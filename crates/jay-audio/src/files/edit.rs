//! The audio editor's edits (stage 6, docs/ARQUITETURA.md section 8). Non-destructive: the original
//! stays as it was read, the edits are a list, and the result is rendered from the original each
//! time, so undo is dropping the last edit. Positions are frames of the audio as it is *after* the
//! edits before (what the user sees when choosing the selection).
//! shortcut: the whole file is in memory (~1.4 GB an hour in stereo), so the editor takes 30 min at
//! most; conversion streams and has no such limit.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::decode::Reader;

/// Longer files don't open in the editor.
pub const MAX_SECONDS: u64 = 30 * 60;

/// Audio in memory: interleaved f32.
#[derive(Debug, Clone, PartialEq)]
pub struct Audio {
    pub rate: u32,
    pub channels: usize,
    pub samples: Vec<f32>,
}

impl Audio {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels
    }

    pub fn read(path: &Path) -> Result<Audio, String> {
        let mut reader = Reader::open(path)?;
        let (rate, channels) = (reader.rate, reader.channels);
        let max = MAX_SECONDS as usize * rate as usize * channels;
        let mut samples = Vec::new();
        while let Some(block) = reader.next_block()? {
            samples.extend(block);
            if samples.len() > max {
                return Err("longer than 30 minutes: convert it, or cut it into parts first".into());
            }
        }
        if samples.is_empty() {
            return Err("no audio in this file".into());
        }
        Ok(Audio { rate, channels, samples })
    }
}

/// One edit on frames `from..to` (clamped to the audio; `to` past the end = the end).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Edit {
    /// Removes the frames.
    Delete { from: usize, to: usize },
    /// Keeps only the frames (crop).
    Keep { from: usize, to: usize },
    /// From silence up to full level.
    FadeIn { from: usize, to: usize },
    /// From full level down to silence.
    FadeOut { from: usize, to: usize },
    Gain { from: usize, to: usize, db: f32 },
    /// The range's peak to −1 dBFS.
    Normalize { from: usize, to: usize },
    Reverse { from: usize, to: usize },
    /// Removes the beginning and end quieter than `db` dBFS.
    TrimSilence { db: f32 },
}

/// The original with every edit applied, in order.
pub fn render(original: &Audio, edits: &[Edit]) -> Audio {
    let mut audio = original.clone();
    for edit in edits {
        apply(&mut audio, *edit);
    }
    audio
}

fn apply(audio: &mut Audio, edit: Edit) {
    let ch = audio.channels;
    let frames = audio.frames();
    let range = |from: usize, to: usize| {
        let to = to.min(frames);
        from.min(to)..to
    };
    let samples = |r: std::ops::Range<usize>| r.start * ch..r.end * ch;
    match edit {
        Edit::Delete { from, to } => {
            audio.samples.drain(samples(range(from, to)));
        }
        Edit::Keep { from, to } => {
            let keep = samples(range(from, to));
            audio.samples.truncate(keep.end);
            audio.samples.drain(..keep.start);
        }
        Edit::FadeIn { from, to } | Edit::FadeOut { from, to } => {
            let r = range(from, to);
            let n = r.len().max(1) as f32;
            for (i, frame) in audio.samples[samples(r)].chunks_exact_mut(ch).enumerate() {
                let x = i as f32 / n;
                // Equal power: sounds even, unlike a linear ramp.
                let g = if matches!(edit, Edit::FadeIn { .. }) { (x * std::f32::consts::FRAC_PI_2).sin() } else { (x * std::f32::consts::FRAC_PI_2).cos() };
                frame.iter_mut().for_each(|s| *s *= g);
            }
        }
        Edit::Gain { from, to, db } => {
            let g = 10f32.powf(db / 20.0);
            for s in &mut audio.samples[samples(range(from, to))] {
                *s = (*s * g).clamp(-1.0, 1.0);
            }
        }
        Edit::Normalize { from, to } => {
            let part = &mut audio.samples[samples(range(from, to))];
            let peak = part.iter().fold(0.0f32, |p, s| p.max(s.abs()));
            if peak > 0.0 {
                let g = 0.891 / peak;
                part.iter_mut().for_each(|s| *s *= g);
            }
        }
        Edit::Reverse { from, to } => {
            let part = &mut audio.samples[samples(range(from, to))];
            let n = part.len() / ch;
            for i in 0..n / 2 {
                for c in 0..ch {
                    part.swap(i * ch + c, (n - 1 - i) * ch + c);
                }
            }
        }
        Edit::TrimSilence { db } => {
            let floor = 10f32.powf(db / 20.0);
            let loud = |f: &[f32]| f.iter().any(|s| s.abs() > floor);
            let all: Vec<&[f32]> = audio.samples.chunks_exact(ch).collect();
            let start = all.iter().position(|f| loud(f)).unwrap_or(0);
            let end = all.iter().rposition(|f| loud(f)).map_or(start, |i| i + 1);
            apply(audio, Edit::Keep { from: start, to: end });
        }
    }
}

/// As a soundboard clip: 48 kHz stereo (for the preview, which plays through a soundboard mixer).
pub fn to_clip(audio: &Audio) -> Result<Vec<f32>, String> {
    let ch = audio.channels;
    let stereo: Vec<f32> = audio.samples.chunks_exact(ch).flat_map(|f| [f[0], f[if ch > 1 { 1 } else { 0 }]]).collect();
    if audio.rate == crate::SAMPLE_RATE || stereo.is_empty() { Ok(stereo) } else { super::decode::resample(&stereo, audio.rate) }
}

/// A 32-bit float WAV, for exporting through the converter (files::convert).
pub fn write_wav(audio: &Audio, path: &Path) -> Result<(), String> {
    let spec = hound::WavSpec { channels: audio.channels as u16, sample_rate: audio.rate, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut wav = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for &s in &audio.samples {
        wav.write_sample(s).map_err(|e| e.to_string())?;
    }
    wav.finalize().map_err(|e| e.to_string())
}

/// The waveform: (min, max) of all channels in each of `columns` slices.
pub fn peaks(audio: &Audio, columns: usize) -> Vec<(f32, f32)> {
    let frames = audio.frames();
    let columns = columns.clamp(1, 8192);
    (0..columns)
        .map(|c| {
            let (a, b) = (frames * c / columns, (frames * (c + 1) / columns).max(frames * c / columns + 1).min(frames));
            audio.samples[a * audio.channels..b * audio.channels].iter().fold((0.0f32, 0.0f32), |(lo, hi), &s| (lo.min(s), hi.max(s)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stereo, frame i = (i, -i) / 1000, so positions are easy to read back.
    fn ramp(frames: usize) -> Audio {
        Audio { rate: 1000, channels: 2, samples: (0..frames).flat_map(|i| [i as f32 / 1000.0, -(i as f32) / 1000.0]).collect() }
    }

    fn left(a: &Audio) -> Vec<f32> {
        a.samples.iter().step_by(2).map(|s| (s * 1000.0).round()).collect()
    }

    #[test]
    fn delete_keep_and_undo() {
        let original = ramp(10);
        let mut edits = vec![Edit::Delete { from: 2, to: 5 }];
        assert_eq!(left(&render(&original, &edits)), [0., 1., 5., 6., 7., 8., 9.]);
        // Positions refer to the audio after the edits before.
        edits.push(Edit::Keep { from: 1, to: 4 });
        assert_eq!(left(&render(&original, &edits)), [1., 5., 6.]);
        edits.pop(); // undo
        assert_eq!(left(&render(&original, &edits)), [0., 1., 5., 6., 7., 8., 9.]);
        edits.pop();
        assert_eq!(render(&original, &edits), original, "the original is untouched");
        // Out-of-range positions are clamped, never a panic.
        assert_eq!(render(&original, &[Edit::Delete { from: 8, to: 999 }]).frames(), 8);
        assert_eq!(render(&original, &[Edit::Keep { from: 50, to: 60 }]).frames(), 0);
    }

    #[test]
    fn fades_gain_normalize_and_reverse() {
        let flat = Audio { rate: 1000, channels: 1, samples: vec![0.5; 100] };
        let faded = render(&flat, &[Edit::FadeIn { from: 0, to: 50 }, Edit::FadeOut { from: 50, to: 100 }]);
        assert_eq!(faded.samples[0], 0.0, "starts silent");
        assert!((faded.samples[49] - 0.5).abs() < 0.01 && (faded.samples[50] - 0.5).abs() < 0.001, "full in the middle");
        assert!(faded.samples[99] < 0.02, "ends silent");
        assert!(faded.samples[..50].windows(2).all(|w| w[1] >= w[0]), "rises smoothly");

        let louder = render(&flat, &[Edit::Gain { from: 0, to: 10, db: 6.0 }]);
        assert!((louder.samples[0] - 0.998).abs() < 0.01 && louder.samples[10] == 0.5);
        let normal = render(&flat, &[Edit::Normalize { from: 0, to: 100 }]);
        assert!(normal.samples.iter().all(|&s| (s - 0.891).abs() < 1e-6));

        let reversed = render(&ramp(5), &[Edit::Reverse { from: 1, to: 4 }]);
        assert_eq!(left(&reversed), [0., 3., 2., 1., 4.]);
        assert!(reversed.samples.as_chunks::<2>().0.iter().all(|[l, r]| *l == -*r), "channels stay together");
    }

    #[test]
    fn trim_silence_keeps_the_sound() {
        let mut samples = vec![0.0; 30];
        samples.extend([0.2; 40]);
        samples.extend(vec![0.0005; 30]); // under −50 dBFS
        let audio = Audio { rate: 1000, channels: 1, samples };
        let trimmed = render(&audio, &[Edit::TrimSilence { db: -50.0 }]);
        assert_eq!(trimmed.samples, vec![0.2; 40]);
        let silent = Audio { rate: 1000, channels: 1, samples: vec![0.0; 10] };
        assert_eq!(render(&silent, &[Edit::TrimSilence { db: -50.0 }]).frames(), 0);
    }

    #[test]
    fn peaks_per_column() {
        let p = peaks(&ramp(1000), 10);
        assert_eq!(p.len(), 10);
        assert!((p[0].1 - 0.099).abs() < 1e-6 && (p[0].0 + 0.099).abs() < 1e-6);
        assert!((p[9].1 - 0.999).abs() < 1e-6);
        assert_eq!(peaks(&ramp(3), 10).len(), 10, "more columns than frames still works");
    }
}
