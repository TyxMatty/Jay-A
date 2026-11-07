//! Audio files to samples. `Reader` reads any file block by block (for conversion, where files can
//! be hours long); `decode` reads a short one whole into Jay-A's format, 48 kHz stereo f32
//! interleaved (the soundboard, docs/ARQUITETURA.md section 7).
//!
//! symphonia (pure Rust) reads WAV, FLAC, MP3, OGG Vorbis, AAC/M4A, AIFF and ALAC; Opus (which
//! symphonia 0.5 doesn't decode) is read with `ogg` + libopus. Files come from the user, so
//! anything odd is an error, never a panic.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::SAMPLE_RATE;

/// An audio file read block by block: interleaved f32 at the file's rate and channels.
pub struct Reader {
    pub rate: u32,
    pub channels: usize,
    /// Frames in the file, when its header says (for progress).
    pub frames: Option<u64>,
    inner: Inner,
}

enum Inner {
    Symphonia { format: Box<dyn FormatReader>, decoder: Box<dyn Decoder>, track: u32 },
    Opus { packets: ogg::PacketReader<BufReader<File>>, decoder: opus::Decoder, skip: usize, out: Vec<f32> },
}

impl Reader {
    pub fn open(path: &Path) -> Result<Reader, String> {
        let is_opus = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("opus"));
        if is_opus { open_opus(path) } else { open_symphonia(path) }
    }

    /// The next block, or None at the end.
    pub fn next_block(&mut self) -> Result<Option<Vec<f32>>, String> {
        let channels = self.channels;
        match &mut self.inner {
            Inner::Symphonia { format, decoder, track } => loop {
                let packet = match format.next_packet() {
                    Ok(packet) => packet,
                    Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
                    Err(Error::ResetRequired) => return Ok(None),
                    Err(e) => return Err(e.to_string()),
                };
                if packet.track_id() != *track {
                    continue;
                }
                let decoded = match decoder.decode(&packet) {
                    Ok(decoded) => decoded,
                    Err(Error::DecodeError(_)) => continue, // a damaged packet: skip it, like players do
                    Err(e) => return Err(e.to_string()),
                };
                let spec = *decoded.spec();
                if spec.channels.count() != channels {
                    return Err("the number of channels changes inside the file".into());
                }
                let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                samples.copy_interleaved_ref(decoded);
                return Ok(Some(clean(samples.samples().to_vec())));
            },
            Inner::Opus { packets, decoder, skip, out } => loop {
                let Some(packet) = packets.read_packet().map_err(|e| e.to_string())? else { return Ok(None) };
                let frames = match decoder.decode_float(&packet.data, out, false) {
                    Ok(frames) => frames,
                    Err(_) => continue, // a damaged packet
                };
                let dropped = (*skip).min(frames); // the encoder's warm-up (pre-skip)
                *skip -= dropped;
                if dropped < frames {
                    return Ok(Some(clean(out[dropped * channels..frames * channels].to_vec())));
                }
            },
        }
    }
}

/// NaN, infinity and overs out: they would hurt the call's limiter, an encoder or the ears.
fn clean(mut samples: Vec<f32>) -> Vec<f32> {
    for s in &mut samples {
        *s = if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 };
    }
    samples
}

fn open_symphonia(path: &Path) -> Result<Reader, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, stream, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("not an audio file I can read ({e})"))?;
    let format = probed.format;
    let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL).ok_or("no audio in this file")?;
    let params = &track.codec_params;
    let rate = params.sample_rate.filter(|&r| r > 0).ok_or("unknown sample rate")?;
    let channels = params.channels.map(|c| c.count()).filter(|&c| c > 0).ok_or("unknown channels")?;
    let decoder = symphonia::default::get_codecs().make(params, &DecoderOptions::default()).map_err(|e| e.to_string())?;
    let (track, frames) = (track.id, params.n_frames);
    Ok(Reader { rate, channels, frames, inner: Inner::Symphonia { format, decoder, track } })
}

/// Ogg Opus (RFC 7845): an OpusHead packet, an OpusTags packet, then audio, always at 48 kHz.
fn open_opus(path: &Path) -> Result<Reader, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut packets = ogg::PacketReader::new(BufReader::new(file));
    let head = packets.read_packet().map_err(|e| format!("not an Opus file ({e})"))?.ok_or("empty file")?.data;
    if head.len() < 19 || &head[..8] != b"OpusHead" {
        return Err("not an Opus file".into());
    }
    let channels = match head[9] {
        1 => opus::Channels::Mono,
        2 => opus::Channels::Stereo,
        n => return Err(format!("Opus with {n} channels is not supported")),
    };
    let skip = u16::from_le_bytes([head[10], head[11]]) as usize;
    packets.read_packet().map_err(|e| e.to_string())?.ok_or("no OpusTags")?;
    let decoder = opus::Decoder::new(SAMPLE_RATE, channels).map_err(|e| e.to_string())?;
    let n = head[9] as usize;
    // Room for 120 ms, the longest Opus packet.
    let out = vec![0.0; 5760 * n];
    Ok(Reader { rate: SAMPLE_RATE, channels: n, frames: None, inner: Inner::Opus { packets, decoder, skip, out } })
}

/// Decodes `path` to 48 kHz stereo (mono files are doubled, more channels keep the first two).
/// Longer than `max_seconds` is an error, found while decoding, so a huge file isn't read whole.
pub fn decode(path: &Path, max_seconds: f32) -> Result<Vec<f32>, String> {
    let mut reader = Reader::open(path)?;
    let (rate, channels) = (reader.rate, reader.channels);
    let max_frames = (max_seconds * rate as f32) as usize;
    let mut stereo: Vec<f32> = Vec::new();
    while let Some(block) = reader.next_block()? {
        for frame in block.chunks_exact(channels) {
            stereo.extend_from_slice(&[frame[0], frame[if channels > 1 { 1 } else { 0 }]]);
        }
        if stereo.len() / 2 > max_frames {
            return Err(format!("longer than {max_seconds} s"));
        }
    }
    if stereo.is_empty() {
        return Err("no audio in this file".into());
    }
    if rate == SAMPLE_RATE { Ok(stereo) } else { Ok(clean(resample(&stereo, rate)?)) }
}

pub(crate) fn resample(stereo: &[f32], rate: u32) -> Result<Vec<f32>, String> {
    let frames = stereo.len() / 2;
    let mut resampler = Fft::<f32>::new(rate as usize, SAMPLE_RATE as usize, 1024, 2, FixedSync::Input).map_err(|e| e.to_string())?;
    let input = InterleavedSlice::new(stereo, 2, frames).map_err(|e| e.to_string())?;
    let output = resampler.process_all(&input, frames, None).map_err(|e| e.to_string())?;
    Ok(output.take_data())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 16-bit PCM WAV of `channels` × `samples` (interleaved).
    fn wav(name: &str, rate: u32, channels: u16, samples: &[f32]) -> std::path::PathBuf {
        let data: Vec<u8> = samples.iter().flat_map(|s| ((s * 32767.0) as i16).to_le_bytes()).collect();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
        bytes.extend_from_slice(&(channels * 2).to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&data);
        let path = std::env::temp_dir().join(format!("jaya-decode-{}-{name}.wav", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn sine(rate: u32, freq: f32, seconds: f32, amp: f32) -> Vec<f32> {
        (0..(rate as f32 * seconds) as usize).map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin()).collect()
    }

    /// Amplitude of `freq` in `x` (Goertzel).
    fn tone(x: &[f32], freq: f32) -> f32 {
        let coeff = 2.0 * (2.0 * std::f64::consts::PI * freq as f64 / SAMPLE_RATE as f64).cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &v in x {
            let s = v as f64 + coeff * s1 - s2;
            (s2, s1) = (s1, s);
        }
        (2.0 * (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0).sqrt() / x.len() as f64) as f32
    }

    #[test]
    fn a_mono_44k_file_becomes_48k_stereo_with_the_same_tone() {
        let path = wav("mono44", 44_100, 1, &sine(44_100, 1000.0, 1.0, 0.5));
        let out = decode(&path, 20.0).unwrap();
        std::fs::remove_file(&path).ok();
        let frames = out.len() / 2;
        assert!((frames as i64 - 48_000).abs() < 50, "{frames} frames for 1 s");
        let (left, right): (Vec<f32>, Vec<f32>) = out.as_chunks::<2>().0.iter().map(|&[l, r]| (l, r)).unzip();
        assert_eq!(left, right, "mono doubled");
        let middle = &left[4_800..43_200];
        assert!((tone(middle, 1000.0) - 0.5).abs() < 0.02, "1 kHz kept: {}", tone(middle, 1000.0));
        assert!(tone(middle, 1088.4) < 0.01, "and not shifted (a wrong rate would move it)");
    }

    #[test]
    fn stereo_48k_passes_through_and_keeps_its_channels() {
        let left = sine(48_000, 440.0, 0.5, 0.3);
        let interleaved: Vec<f32> = left.iter().flat_map(|&l| [l, 0.0]).collect();
        let path = wav("stereo48", 48_000, 2, &interleaved);
        let out = decode(&path, 20.0).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(out.len(), interleaved.len());
        assert!(out.as_chunks::<2>().0.iter().all(|[_, r]| *r == 0.0), "the right channel stays silent");
    }

    #[test]
    fn too_long_and_not_audio_are_errors() {
        let path = wav("long", 8_000, 1, &vec![0.1; 8_000 * 3]);
        assert_eq!(decode(&path, 2.0).unwrap_err(), "longer than 2 s");
        std::fs::remove_file(&path).ok();
        let junk = std::env::temp_dir().join(format!("jaya-decode-{}-junk.mp3", std::process::id()));
        std::fs::write(&junk, b"definitely not audio, just text pretending").unwrap();
        assert!(decode(&junk, 20.0).is_err());
        std::fs::remove_file(&junk).ok();
        assert!(decode(Path::new("no-such-file.wav"), 20.0).is_err());
    }
}
