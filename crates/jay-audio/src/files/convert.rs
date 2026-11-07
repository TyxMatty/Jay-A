//! Converting one file (stage 5, docs/ARQUITETURA.md section 9): read block by block, map the
//! channels, resample, apply the normalization gain, encode. Nothing holds the whole file, except
//! FLAC's encoder, which keeps the *encoded* stream until the end.
//! shortcut: FLAC output is built in memory (about half the PCM size); stream it if long FLACs matter.
//!
//! The output is written to `<name>.<ext>.tmp` and renamed at the end; an error or a cancel removes
//! the `.tmp`, so a half-written file never has the real name.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::num::{NonZeroU8, NonZeroU32};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};

use super::decode::Reader;

/// Bigger inputs are refused (they are the user's files: untrusted).
const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Longer inputs are refused.
const MAX_SECONDS: u64 = 10 * 60 * 60;
/// Normalizing brings the peak here (−1 dBFS).
const PEAK: f32 = 0.891;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Format {
    /// 16 or 24-bit integer, or 32-bit float.
    Wav { bits: u16 },
    Flac,
    Mp3 { kbps: u32 },
    /// Vorbis quality, −0.1 to 1.0.
    Ogg { quality: f32 },
    Opus { kbps: u32 },
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Wav { .. } => "wav",
            Format::Flac => "flac",
            Format::Mp3 { .. } => "mp3",
            Format::Ogg { .. } => "ogg",
            Format::Opus { .. } => "opus",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub format: Format,
    /// None = the file's (when the format allows it).
    pub rate: Option<u32>,
    /// 1 or 2; None = the file's (more than 2 become 2).
    pub channels: Option<usize>,
    /// Bring the peak to −1 dBFS (a first pass finds it).
    pub normalize: bool,
}

/// Converts `input` to `output`. `progress` gets 0..1 when the file's length is known.
pub fn convert(input: &Path, output: &Path, options: Options, cancel: &AtomicBool, progress: &mut dyn FnMut(f32)) -> Result<(), String> {
    if fs::metadata(input).map_err(|e| e.to_string())?.len() > MAX_BYTES {
        return Err("larger than 2 GB".into());
    }
    let gain = if options.normalize { normalization(input, cancel)? } else { 1.0 };
    let reader = Reader::open(input)?;
    if reader.frames.is_some_and(|f| f / reader.rate as u64 > MAX_SECONDS) {
        return Err("longer than 10 hours".into());
    }
    let rate = match options.format {
        Format::Opus { .. } => 48_000, // Opus is always 48 kHz
        // LAME's rates; anything else becomes 48 kHz.
        Format::Mp3 { .. } => Some(options.rate.unwrap_or(reader.rate))
            .filter(|r| [8_000, 11_025, 12_000, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000].contains(r))
            .unwrap_or(48_000),
        _ => options.rate.unwrap_or(reader.rate),
    };
    let channels = options.channels.unwrap_or(reader.channels).clamp(1, 2);
    let mut blocks = Blocks::new(reader, channels, rate, gain, cancel, progress)?;
    let tmp = PathBuf::from(format!("{}.tmp", output.display()));
    let written = encode(&mut blocks, options.format, &tmp);
    match written.and_then(|()| fs::rename(&tmp, output).map_err(|e| e.to_string())) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// The gain that brings the file's peak to −1 dBFS (1 for silence).
fn normalization(input: &Path, cancel: &AtomicBool) -> Result<f32, String> {
    let mut reader = Reader::open(input)?;
    let mut peak = 0.0f32;
    while let Some(block) = reader.next_block()? {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        peak = block.iter().fold(peak, |p, s| p.max(s.abs()));
    }
    Ok(if peak > 0.0 { PEAK / peak } else { 1.0 })
}

/// The converted audio, block by block: interleaved f32 at the output's rate and channels.
struct Blocks<'a> {
    reader: Reader,
    channels: usize,
    rate: u32,
    resampler: Option<Stretch>,
    gain: f32,
    done: bool,
    read: u64,
    cancel: &'a AtomicBool,
    progress: &'a mut dyn FnMut(f32),
}

impl<'a> Blocks<'a> {
    fn new(reader: Reader, channels: usize, rate: u32, gain: f32, cancel: &'a AtomicBool, progress: &'a mut dyn FnMut(f32)) -> Result<Blocks<'a>, String> {
        let resampler = (reader.rate != rate).then(|| Stretch::new(reader.rate, rate, channels)).transpose()?;
        Ok(Blocks { reader, channels, rate, resampler, gain, done: false, read: 0, cancel, progress })
    }

    fn next(&mut self) -> Result<Option<Vec<f32>>, String> {
        while !self.done {
            if self.cancel.load(Ordering::SeqCst) {
                return Err("cancelled".into());
            }
            let mut out = Vec::new();
            match self.reader.next_block()? {
                Some(block) => {
                    let mapped = map_channels(&block, self.reader.channels, self.channels);
                    self.read += (mapped.len() / self.channels) as u64;
                    if self.read / self.reader.rate as u64 > MAX_SECONDS {
                        return Err("longer than 10 hours".into());
                    }
                    if let Some(total) = self.reader.frames.filter(|&t| t > 0) {
                        (self.progress)((self.read as f32 / total as f32).min(1.0));
                    }
                    match &mut self.resampler {
                        Some(r) => r.push(&mapped, &mut out)?,
                        None => out = mapped,
                    }
                }
                None => {
                    self.done = true;
                    if let Some(r) = &mut self.resampler {
                        r.finish(&mut out)?;
                    }
                }
            }
            if !out.is_empty() {
                for s in &mut out {
                    *s = (*s * self.gain).clamp(-1.0, 1.0);
                }
                return Ok(Some(out));
            }
        }
        Ok(None)
    }
}

/// `from` channels to `to` (1 or 2): mono is the average of the first two, stereo doubles mono.
fn map_channels(block: &[f32], from: usize, to: usize) -> Vec<f32> {
    if from == to {
        return block.to_vec();
    }
    let second = if from > 1 { 1 } else { 0 };
    let frames = block.chunks_exact(from);
    if to == 1 {
        frames.map(|f| (f[0] + f[second]) * 0.5).collect()
    } else {
        frames.flat_map(|f| [f[0], f[second]]).collect()
    }
}

/// A streaming resampler: blocks in, blocks out, the delay trimmed and the length kept.
struct Stretch {
    resampler: Fft<f32>,
    channels: usize,
    pending: Vec<f32>,
    out: Vec<f32>,
    /// Output frames still to drop (the resampler's delay).
    delay: usize,
    ratio: f64,
    read: u64,
    written: u64,
}

impl Stretch {
    fn new(from: u32, to: u32, channels: usize) -> Result<Stretch, String> {
        let resampler = Fft::<f32>::new(from as usize, to as usize, 1024, channels, FixedSync::Input).map_err(|e| e.to_string())?;
        let out = vec![0.0; resampler.output_frames_max() * channels];
        let delay = resampler.output_delay();
        Ok(Stretch { resampler, channels, pending: Vec::new(), out, delay, ratio: to as f64 / from as f64, read: 0, written: 0 })
    }

    fn push(&mut self, block: &[f32], sink: &mut Vec<f32>) -> Result<(), String> {
        self.read += (block.len() / self.channels) as u64;
        self.pending.extend_from_slice(block);
        self.run(sink)
    }

    fn run(&mut self, sink: &mut Vec<f32>) -> Result<(), String> {
        let ch = self.channels;
        loop {
            let need = self.resampler.input_frames_next();
            if self.pending.len() < need * ch {
                return Ok(());
            }
            let frames = self.out.len() / ch;
            let input = InterleavedSlice::new(&self.pending[..need * ch], ch, need).map_err(|e| e.to_string())?;
            let mut output = InterleavedSlice::new_mut(&mut self.out, ch, frames).map_err(|e| e.to_string())?;
            let (used, made) = self.resampler.process_into_buffer(&input, &mut output, None).map_err(|e| e.to_string())?;
            self.pending.drain(..used * ch);
            let skip = self.delay.min(made);
            self.delay -= skip;
            sink.extend_from_slice(&self.out[skip * ch..made * ch]);
            self.written += (made - skip) as u64;
        }
    }

    /// The rest, padded with silence through the resampler, cut to the input's length × ratio.
    fn finish(&mut self, sink: &mut Vec<f32>) -> Result<(), String> {
        let expected = (self.read as f64 * self.ratio).round() as u64;
        let before = self.written;
        while self.written < expected {
            let need = self.resampler.input_frames_next() * self.channels;
            self.pending.resize(self.pending.len().max(need), 0.0);
            self.run(sink)?;
        }
        sink.truncate(((expected - before) as usize) * self.channels);
        Ok(())
    }
}

fn encode(blocks: &mut Blocks, format: Format, tmp: &Path) -> Result<(), String> {
    let (rate, channels) = (blocks.rate, blocks.channels);
    match format {
        Format::Wav { bits } => {
            let spec = hound::WavSpec {
                channels: channels as u16,
                sample_rate: rate,
                bits_per_sample: bits,
                sample_format: if bits == 32 { hound::SampleFormat::Float } else { hound::SampleFormat::Int },
            };
            let mut wav = hound::WavWriter::create(tmp, spec).map_err(|e| e.to_string())?;
            let full = ((1i64 << (bits.min(24) - 1)) - 1) as f32;
            while let Some(block) = blocks.next()? {
                for s in block {
                    let written = if bits == 32 { wav.write_sample(s) } else { wav.write_sample((s * full).round() as i32) };
                    written.map_err(|e| e.to_string())?;
                }
            }
            wav.finalize().map_err(|e| e.to_string())
        }
        Format::Flac => flac(blocks, tmp),
        Format::Mp3 { kbps } => mp3(blocks, kbps, tmp),
        Format::Ogg { quality } => {
            let file = BufWriter::new(File::create(tmp).map_err(|e| e.to_string())?);
            let rate = NonZeroU32::new(rate).ok_or("rate 0")?;
            let ch = NonZeroU8::new(channels as u8).ok_or("no channels")?;
            let mut builder = vorbis_rs::VorbisEncoderBuilder::new(rate, ch, file).map_err(|e| e.to_string())?;
            builder.bitrate_management_strategy(vorbis_rs::VorbisBitrateManagementStrategy::QualityVbr { target_quality: quality.clamp(-0.1, 1.0) });
            let mut vorbis = builder.build().map_err(|e| e.to_string())?;
            while let Some(block) = blocks.next()? {
                let planar: Vec<Vec<f32>> = (0..channels).map(|c| block.iter().skip(c).step_by(channels).copied().collect()).collect();
                vorbis.encode_audio_block(&planar).map_err(|e| e.to_string())?;
            }
            vorbis.finish().map_err(|e| e.to_string())?.flush().map_err(|e| e.to_string())
        }
        Format::Opus { kbps } => opus_ogg(blocks, kbps, tmp),
    }
}

fn mp3(blocks: &mut Blocks, kbps: u32, tmp: &Path) -> Result<(), String> {
    use mp3lame_encoder::{Bitrate, Builder, FlushNoGap, InterleavedPcm, MonoPcm, Quality};
    let bitrate = match kbps {
        ..=96 => Bitrate::Kbps96,
        97..=128 => Bitrate::Kbps128,
        129..=160 => Bitrate::Kbps160,
        161..=192 => Bitrate::Kbps192,
        193..=256 => Bitrate::Kbps256,
        _ => Bitrate::Kbps320,
    };
    let mut builder = Builder::new().ok_or("LAME didn't start")?;
    builder.set_num_channels(blocks.channels as u8).map_err(|e| e.to_string())?;
    builder.set_sample_rate(blocks.rate).map_err(|e| e.to_string())?;
    builder.set_brate(bitrate).map_err(|e| e.to_string())?;
    builder.set_quality(Quality::Good).map_err(|e| e.to_string())?;
    let mut lame = builder.build().map_err(|e| e.to_string())?;
    let mut file = BufWriter::new(File::create(tmp).map_err(|e| e.to_string())?);
    let mut out = Vec::new();
    while let Some(block) = blocks.next()? {
        out.clear();
        out.reserve(mp3lame_encoder::max_required_buffer_size(block.len()));
        let written = if blocks.channels == 1 { lame.encode_to_vec(MonoPcm(&block), &mut out) } else { lame.encode_to_vec(InterleavedPcm(&block), &mut out) };
        written.map_err(|e| e.to_string())?;
        file.write_all(&out).map_err(|e| e.to_string())?;
    }
    out.clear();
    out.reserve(7200);
    lame.flush_to_vec::<FlushNoGap>(&mut out).map_err(|e| e.to_string())?;
    file.write_all(&out).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())
}

/// FLAC, 16-bit: flacenc pulls the samples through this source.
fn flac(blocks: &mut Blocks, tmp: &Path) -> Result<(), String> {
    use flacenc::bitsink::ByteSink;
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;

    struct Source<'b, 'c> {
        blocks: &'b mut Blocks<'c>,
        pending: Vec<i32>,
        error: Option<String>,
    }
    impl flacenc::source::Source for Source<'_, '_> {
        fn channels(&self) -> usize {
            self.blocks.channels
        }
        fn bits_per_sample(&self) -> usize {
            16
        }
        fn sample_rate(&self) -> usize {
            self.blocks.rate as usize
        }
        fn read_samples<F: flacenc::source::Fill>(&mut self, block_size: usize, dest: &mut F) -> Result<usize, flacenc::error::SourceError> {
            let want = block_size * self.blocks.channels;
            while self.pending.len() < want && self.error.is_none() {
                match self.blocks.next() {
                    Ok(Some(block)) => self.pending.extend(block.iter().map(|s| (s * 32767.0).round() as i32)),
                    Ok(None) => break,
                    Err(e) => self.error = Some(e),
                }
            }
            let take = want.min(self.pending.len()) / self.blocks.channels * self.blocks.channels;
            let chunk: Vec<i32> = self.pending.drain(..take).collect();
            dest.fill_interleaved(&chunk)?;
            Ok(take / self.blocks.channels)
        }
    }

    let config = flacenc::config::Encoder::default().into_verified().map_err(|(_, e)| e.to_string())?;
    let mut source = Source { blocks, pending: Vec::new(), error: None };
    let stream = flacenc::encode_with_fixed_block_size(&config, &mut source, 4096).map_err(|e| e.to_string())?;
    if let Some(error) = source.error {
        return Err(error);
    }
    let mut sink = ByteSink::new();
    stream.write(&mut sink).map_err(|e| e.to_string())?;
    let mut bytes = sink.as_slice().to_vec();
    // STREAMINFO's minimum block size must leave out the last (shorter) block, and flacenc counts it:
    // readers then refuse the file. Fixed-size blocks: the minimum is the maximum.
    if bytes.len() > 12 && &bytes[..4] == b"fLaC" {
        bytes.copy_within(10..12, 8);
    }
    fs::write(tmp, bytes).map_err(|e| e.to_string())
}

/// Ogg Opus (RFC 7845): OpusHead, OpusTags, then 20 ms packets; the last page's granule position
/// trims the padding of the last packet.
fn opus_ogg(blocks: &mut Blocks, kbps: u32, tmp: &Path) -> Result<(), String> {
    use ogg::writing::PacketWriteEndInfo::{EndPage, EndStream, NormalPacket};
    const FRAME: usize = 960;
    let ch = blocks.channels;
    let channels = if ch == 1 { opus::Channels::Mono } else { opus::Channels::Stereo };
    let mut encoder = opus::Encoder::new(48_000, channels, opus::Application::Audio).map_err(|e| e.to_string())?;
    encoder.set_bitrate(opus::Bitrate::Bits(kbps as i32 * 1000)).map_err(|e| e.to_string())?;
    let pre_skip = encoder.get_lookahead().map_err(|e| e.to_string())? as u64;
    let serial = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(1);
    let mut writer = ogg::PacketWriter::new(BufWriter::new(File::create(tmp).map_err(|e| e.to_string())?));

    let mut head = b"OpusHead".to_vec();
    head.push(1); // version
    head.push(ch as u8);
    head.extend_from_slice(&(pre_skip as u16).to_le_bytes());
    head.extend_from_slice(&48_000u32.to_le_bytes()); // the input's rate, for information
    head.extend_from_slice(&0i16.to_le_bytes()); // output gain
    head.push(0); // channel mapping family 0: mono or stereo
    writer.write_packet(head, serial, EndPage, 0).map_err(|e| e.to_string())?;
    let mut tags = b"OpusTags".to_vec();
    tags.extend_from_slice(&5u32.to_le_bytes());
    tags.extend_from_slice(b"Jay-A");
    tags.extend_from_slice(&0u32.to_le_bytes()); // no comments
    writer.write_packet(tags, serial, EndPage, 0).map_err(|e| e.to_string())?;

    let mut pending: Vec<f32> = Vec::new();
    let mut total = 0u64; // input frames
    let mut encoded = 0u64; // frames in packets so far
    let mut packet = vec![0u8; 4000];
    let mut last: Option<Vec<u8>> = None;
    let mut ended = false;
    loop {
        if !ended {
            match blocks.next()? {
                Some(block) => {
                    total += (block.len() / ch) as u64;
                    pending.extend(block);
                }
                None => ended = true,
            }
        }
        // After the end, silence until the packets cover the pre-skip and every input frame.
        if ended && pending.len() < FRAME * ch {
            if encoded >= pre_skip + total {
                break;
            }
            pending.resize(FRAME * ch, 0.0);
        }
        while pending.len() >= FRAME * ch {
            let n = encoder.encode_float(&pending[..FRAME * ch], &mut packet).map_err(|e| e.to_string())?;
            pending.drain(..FRAME * ch);
            if let Some(previous) = last.replace(packet[..n].to_vec()) {
                writer.write_packet(previous, serial, NormalPacket, encoded).map_err(|e| e.to_string())?;
            }
            encoded += FRAME as u64;
        }
    }
    let last = last.unwrap_or_default();
    writer.write_packet(last, serial, EndStream, pre_skip + total).map_err(|e| e.to_string())?;
    writer.inner_mut().flush().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::decode::Reader;

    fn wav(name: &str, rate: u32, channels: u16, frames: usize, amp: f32) -> PathBuf {
        let path = std::env::temp_dir().join(format!("jaya-convert-{}-{name}.wav", std::process::id()));
        let spec = hound::WavSpec { channels, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..frames {
            let s = amp * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / rate as f32).sin();
            for _ in 0..channels {
                w.write_sample((s * 32767.0) as i16).unwrap();
            }
        }
        w.finalize().unwrap();
        path
    }

    /// The whole file back: (rate, channels, interleaved samples).
    fn read(path: &Path) -> (u32, usize, Vec<f32>) {
        let mut r = Reader::open(path).unwrap();
        let mut all = Vec::new();
        while let Some(b) = r.next_block().unwrap() {
            all.extend(b);
        }
        (r.rate, r.channels, all)
    }

    /// Amplitude of 1 kHz in the first channel's middle (Goertzel).
    fn tone(samples: &[f32], channels: usize, rate: u32) -> f32 {
        let mono: Vec<f32> = samples.iter().step_by(channels).copied().collect();
        let x = &mono[mono.len() / 4..mono.len() * 3 / 4];
        let coeff = 2.0 * (2.0 * std::f64::consts::PI * 1000.0 / rate as f64).cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &v in x {
            let s = v as f64 + coeff * s1 - s2;
            (s2, s1) = (s1, s);
        }
        (2.0 * (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0).sqrt() / x.len() as f64) as f32
    }

    fn run(input: &Path, format: Format, rate: Option<u32>, channels: Option<usize>, normalize: bool) -> Result<PathBuf, String> {
        let out = input.with_extension(format!("out.{}", format.extension()));
        let cancel = AtomicBool::new(false);
        convert(input, &out, Options { format, rate, channels, normalize }, &cancel, &mut |_| {})?;
        Ok(out)
    }

    #[test]
    fn every_format_keeps_the_tone_and_the_length() {
        let input = wav("tone", 44_100, 2, 44_100, 0.5);
        for (format, rate) in [
            (Format::Wav { bits: 16 }, 44_100),
            (Format::Wav { bits: 24 }, 44_100),
            (Format::Wav { bits: 32 }, 44_100),
            (Format::Flac, 44_100),
            (Format::Mp3 { kbps: 192 }, 44_100),
            (Format::Ogg { quality: 0.5 }, 44_100),
            (Format::Opus { kbps: 128 }, 48_000),
        ] {
            let out = run(&input, format, None, None, false).unwrap();
            let (got_rate, channels, samples) = read(&out);
            let seconds = samples.len() as f32 / channels as f32 / got_rate as f32;
            let level = tone(&samples, channels, got_rate);
            println!("{format:?}: {got_rate} Hz, {channels} ch, {seconds:.3} s, 1 kHz at {level:.3}");
            assert_eq!((got_rate, channels), (rate, 2), "{format:?}");
            assert!((seconds - 1.0).abs() < 0.06, "{format:?}: {seconds} s"); // MP3 adds a frame or two
            assert!((level - 0.5).abs() < 0.03, "{format:?}: the tone at {level}");
            assert!(!PathBuf::from(format!("{}.tmp", out.display())).exists());
            fs::remove_file(out).ok();
        }
        fs::remove_file(input).ok();
    }

    #[test]
    fn resampling_mono_and_normalizing() {
        let input = wav("quiet", 44_100, 2, 44_100, 0.1);
        let out = run(&input, Format::Wav { bits: 32 }, Some(48_000), Some(1), true).unwrap();
        let (rate, channels, samples) = read(&out);
        assert_eq!((rate, channels), (48_000, 1));
        assert!((samples.len() as i64 - 48_000).abs() <= 1, "{} frames", samples.len());
        let peak = samples.iter().fold(0.0f32, |p, s| p.max(s.abs()));
        assert!((peak - PEAK).abs() < 0.01, "peak at −1 dBFS: {peak}");
        assert!((tone(&samples, 1, 48_000) - PEAK).abs() < 0.02);
        fs::remove_file(out).ok();
        fs::remove_file(input).ok();
    }

    #[test]
    fn junk_and_cancel_leave_nothing_behind() {
        let junk = std::env::temp_dir().join(format!("jaya-convert-{}-junk.mp3", std::process::id()));
        fs::write(&junk, b"just text with an audio name").unwrap();
        assert!(run(&junk, Format::Wav { bits: 16 }, None, None, false).is_err());
        assert!(!junk.with_extension("out.wav").exists() && !junk.with_extension("out.wav.tmp").exists());
        fs::remove_file(&junk).ok();

        let input = wav("cancel", 44_100, 2, 44_100 * 3, 0.5);
        let out = input.with_extension("out.flac");
        let cancel = AtomicBool::new(true);
        let result = convert(&input, &out, Options { format: Format::Flac, rate: None, channels: None, normalize: false }, &cancel, &mut |_| {});
        assert_eq!(result, Err("cancelled".into()));
        assert!(!out.exists() && !PathBuf::from(format!("{}.tmp", out.display())).exists());
        fs::remove_file(input).ok();
    }
}
