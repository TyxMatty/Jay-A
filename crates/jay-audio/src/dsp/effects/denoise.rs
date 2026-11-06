//! Noise removal by neural network: turns down whatever isn't voice, including what WebRTC's
//! suppressor (made for steady noise like fans and hiss) lets through: keyboards, mouse clicks,
//! a TV, people talking further away. Two models, like EasyEffects (docs/REFERENCIAS.md, 1.3):
//! - DeepFilterNet 3 (`libDF`, MIT/Apache, the model built in): the strongest; 40 ms of latency.
//! - RNNoise (`nnnoiseless`, BSD): lighter and weaker; 10 ms.
//!
//! `limit` is the most it turns anything down: 100 dB means no limit; lower keeps a bit of the
//! room, which can sound more natural.
//!
//! DeepFilterNet runs on a thread of its own, like DeepFilterNet's own LADSPA plugin: its
//! inference allocates (and its state can't change threads), so it stays off the audio thread
//! (RNF-3). The audio thread hands it each frame and takes back the previous one's result, which
//! costs one frame of latency. A result that isn't ready in time is replaced by the input from the
//! same moment, unfiltered, so the timing never slips. RNNoise allocates nothing and runs inline.

use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;

use df::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::{ArrayView2, ArrayViewMut2};
use nnnoiseless::DenoiseState;

use crate::dsp::FRAME;
use crate::dsp::effect::{Effect, EffectInfo};
use crate::dsp::params::{ParamSpec, Scale, db_to_gain};

const MODELS: &[&str] = &["deepfilternet", "rnnoise"];
const RNNOISE: f32 = 1.0;

static PARAMS: [ParamSpec; 2] = [
    ParamSpec { key: "model", min: 0.0, max: 1.0, default: 0.0, step: 1.0, scale: Scale::Choice(MODELS), unit: "" },
    ParamSpec { key: "limit", min: 6.0, max: 100.0, default: 100.0, step: 1.0, scale: Scale::Linear, unit: "dB" },
];

/// Frames of delay of DeepFilterNet 3 at 48 kHz: hops of 480 samples (our frame), 2 hops of
/// lookahead and one more from its 960-sample window; plus one for the hand-off to its thread.
const DFN_FRAMES: usize = 4;
/// RNNoise: its 960-sample window overlaps one frame.
const RNNOISE_LATENCY: usize = FRAME;

pub const INFO: EffectInfo = EffectInfo {
    kind: "denoise",
    params: &PARAMS,
    create: || Box::new(Denoise::new()),
    // A model change changes the latency, so the chain is rebuilt: models are never created on
    // the audio thread.
    latency: |v| if v[0] == RNNOISE { RNNOISE_LATENCY } else { DFN_FRAMES * FRAME },
};

/// The DeepFilterNet model, read once from the 8 MB archive built into the app; each instance
/// gets a copy.
fn dfn_params() -> DfParams {
    static PARAMS: OnceLock<DfParams> = OnceLock::new();
    PARAMS.get_or_init(DfParams::default).clone()
}

type Frame = [f32; FRAME];

enum Job {
    /// A frame and its number.
    Frame(u64, Frame),
    Limit(f32),
}

/// DeepFilterNet on its own thread (which creates it, so it never changes threads).
struct Dfn {
    jobs: SyncSender<Job>,
    results: Receiver<(u64, Frame)>,
    thread: Option<JoinHandle<()>>,
    /// Frames sent so far.
    sent: u64,
    /// The last DFN_FRAMES inputs: the fallback for a late result (same moment, unfiltered).
    inputs: Box<[Frame; DFN_FRAMES]>,
    /// Frames whose result came too late, after the first second (the model's start-up).
    #[cfg(test)]
    late: u64,
}

impl Dfn {
    fn start(limit: f32) -> Dfn {
        let (jobs, jobs_rx) = sync_channel::<Job>(8);
        let (results_tx, results) = sync_channel(8);
        let thread = std::thread::Builder::new()
            .name("jay-audio-deepfilternet".into())
            .spawn(move || {
                let mut df = match DfTract::new(dfn_params(), &RuntimeParams::default_with_ch(1).with_atten_lim(limit)) {
                    Ok(df) => df,
                    Err(e) => return log::warn!("DeepFilterNet didn't start; the audio passes unfiltered: {e}"),
                };
                let mut out = [0.0; FRAME];
                #[cfg(test)]
                let mut times: Vec<f32> = Vec::new();
                while let Ok(job) = jobs_rx.recv() {
                    match job {
                        Job::Limit(db) => df.set_atten_lim(db),
                        Job::Frame(n, input) => {
                            #[cfg(test)]
                            let started = std::time::Instant::now();
                            let noisy = ArrayView2::from_shape((1, FRAME), &input[..]).expect("1 x FRAME");
                            let clean = ArrayViewMut2::from_shape((1, FRAME), &mut out[..]).expect("1 x FRAME");
                            if df.process(noisy, clean).is_err() {
                                out.fill(0.0);
                            }
                            #[cfg(test)]
                            times.push(started.elapsed().as_secs_f32() * 1000.0);
                            if results_tx.send((n, out)).is_err() {
                                break;
                            }
                        }
                    }
                }
                #[cfg(test)]
                if !times.is_empty() {
                    times.sort_by(f32::total_cmp);
                    let at = |p: usize| times[(times.len() - 1) * p / 100];
                    eprintln!("    DeepFilterNet por bloco: p50 {:.1} ms, p99 {:.1} ms, máx {:.1} ms", at(50), at(99), at(100));
                }
            })
            .ok();
        Dfn {
            jobs,
            results,
            thread,
            sent: 0,
            inputs: Box::new([[0.0; FRAME]; DFN_FRAMES]),
            #[cfg(test)]
            late: 0,
        }
    }

    fn process(&mut self, frame: &mut [f32]) {
        let n = self.sent;
        self.sent += 1;
        let mut input = [0.0; FRAME];
        input.copy_from_slice(frame);
        // The input from DFN_FRAMES frames ago: what the result we want now was made from.
        let fallback = std::mem::replace(&mut self.inputs[n as usize % DFN_FRAMES], input);
        let _ = self.jobs.try_send(Job::Frame(n, input)); // full: that frame gets its fallback
        // The result for the previous frame; older ones came too late and are dropped.
        let mut result = None;
        while let Ok((k, out)) = self.results.try_recv() {
            if k + 1 == n {
                result = Some(out);
                break;
            }
        }
        #[cfg(test)]
        if result.is_none() && n >= 100 {
            self.late += 1;
        }
        frame.copy_from_slice(&result.unwrap_or(fallback));
    }
}

impl Drop for Dfn {
    fn drop(&mut self) {
        #[cfg(test)]
        if self.sent > 100 {
            eprintln!("    DeepFilterNet atrasado: {} de {} blocos depois do primeiro segundo", self.late, self.sent - 100);
        }
        // Ends the thread: replacing our ends closes both queues, so it stops waiting on either.
        self.jobs = sync_channel(0).0;
        self.results = sync_channel(0).1;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

enum Model {
    DeepFilterNet(Dfn),
    /// RNNoise, and the input delayed like its output (for `limit`).
    RNNoise(Box<DenoiseState<'static>>, Box<[f32; RNNOISE_LATENCY]>),
}

struct Denoise {
    model: Model,
    which: f32,
    limit: f32,
    input: Frame,
    output: Frame,
}

impl Denoise {
    fn new() -> Denoise {
        let limit = PARAMS[1].default;
        Denoise { model: Model::DeepFilterNet(Dfn::start(limit)), which: PARAMS[0].default, limit, input: [0.0; FRAME], output: [0.0; FRAME] }
    }
}

impl Effect for Denoise {
    fn set_param(&mut self, index: usize, value: f32) {
        if index == 1 {
            self.limit = value;
            if let Model::DeepFilterNet(dfn) = &self.model {
                let _ = dfn.jobs.try_send(Job::Limit(value));
            }
        } else if value != self.which {
            // Only when the chain is built (see INFO.latency): creating a model allocates.
            self.which = value;
            self.model = if value == RNNOISE {
                Model::RNNoise(DenoiseState::new(), Box::new([0.0; RNNOISE_LATENCY]))
            } else {
                Model::DeepFilterNet(Dfn::start(self.limit))
            };
        }
    }

    fn process(&mut self, frame: &mut [f32]) {
        if frame.len() != FRAME {
            return; // both models work on whole frames; the chain always gives one
        }
        match &mut self.model {
            Model::DeepFilterNet(dfn) => dfn.process(frame),
            Model::RNNoise(state, delayed) => {
                // RNNoise works in 16-bit sample units.
                for (i, s) in self.input.iter_mut().zip(frame.iter()) {
                    *i = s * 32768.0;
                }
                state.process_frame(&mut self.output, &self.input);
                let floor = db_to_gain(-self.limit);
                for (k, s) in frame.iter_mut().enumerate() {
                    let original = std::mem::replace(&mut delayed[k], *s);
                    let denoised = self.output[k] / 32768.0;
                    *s = denoised + floor * (original - denoised);
                }
            }
        }
    }

    fn latency(&self) -> usize {
        (INFO.latency)(&[self.which])
    }

    /// Clears what's cheap to clear. The models keep their state: resetting them allocates, and
    /// this also runs on the audio thread (an effect turned back on). They adapt in a few frames.
    fn reset(&mut self) {
        if let Model::RNNoise(_, delayed) = &mut self.model {
            delayed.fill(0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::dsp::FRAME;
    use crate::dsp::chain::{ChainControl, ChainRunner, ChainSpec, SlotSpec};
    use crate::level_db;

    fn runner(model: f32) -> ChainRunner {
        let mut slot = SlotSpec::new("denoise#0", "denoise").unwrap();
        let i = slot.param_index("model").unwrap();
        slot.values[i] = model;
        ChainControl::new(ChainSpec { slots: vec![slot] }).unwrap().runner()
    }

    /// Runs `input` through `runner` frame by frame. `paced`: one frame every 10 ms, like the
    /// microphone (DeepFilterNet's thread needs the time; faster, its results come too late).
    fn run(runner: &mut ChainRunner, input: &[f32], paced: bool) -> Vec<f32> {
        let start = Instant::now();
        let mut out = Vec::with_capacity(input.len());
        for (k, chunk) in input.chunks(FRAME).enumerate() {
            if paced {
                let due = start + Duration::from_millis(10 * k as u64);
                std::thread::sleep(due.saturating_duration_since(Instant::now()));
            }
            let mut frame = chunk.to_vec();
            frame.resize(FRAME, 0.0);
            runner.process(&mut frame);
            out.extend_from_slice(&frame[..chunk.len()]);
        }
        out
    }

    fn noise(seconds: f32, amplitude: f32) -> Vec<f32> {
        let mut seed = 99u32;
        (0..(seconds * 48_000.0) as usize)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                amplitude * ((seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0)
            })
            .collect()
    }

    #[test]
    fn rnnoise_turns_steady_noise_down() {
        // A fan's rumble (RNNoise barely touches white noise: little of its training looks like it).
        let input: Vec<f32> = fan(96_000).iter().map(|s| s * 0.1).collect();
        let output = run(&mut runner(1.0), &input, false);
        let cut = level_db(&output[48_000..]) - level_db(&input[48_000..]);
        assert!(cut <= -15.0, "RNNoise on a fan: {cut:.1} dB");
    }

    #[test]
    fn deepfilternet_turns_steady_noise_down_on_its_own_thread() {
        let input = noise(2.0, 0.1);
        let output = run(&mut runner(0.0), &input, true);
        let cut = level_db(&output[72_000..]) - level_db(&input[72_000..]);
        assert!(cut <= -15.0, "DeepFilterNet on white noise: {cut:.1} dB");
    }

    #[test]
    fn deepfilternet_keeps_the_audio_thread_free_of_allocations() {
        use crate::dsp::tests::allocations;
        let mut runner = runner(0.0);
        let mut during = 0;
        let start = Instant::now();
        for (k, chunk) in noise(1.0, 0.1).chunks(FRAME).enumerate() {
            std::thread::sleep((start + Duration::from_millis(10 * k as u64)).saturating_duration_since(Instant::now()));
            let mut frame = chunk.to_vec();
            let before = allocations();
            runner.process(&mut frame);
            during += allocations() - before;
        }
        assert_eq!(during, 0, "allocations on the audio thread");
    }

    // ---------- evaluation with speech ----------

    /// 16-bit mono WAV → samples.
    fn read_wav(path: &std::path::Path) -> Vec<f32> {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut at = 12;
        while at + 8 <= bytes.len() {
            let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
            if &bytes[at..at + 4] == b"data" {
                let data = &bytes[at + 8..(at + 8 + size).min(bytes.len())];
                return data.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect();
            }
            at += 8 + size + size % 2;
        }
        panic!("{}: no data chunk", path.display());
    }

    /// Keyboard: a press and a release click every 120-300 ms, each a short decaying burst.
    fn keyboard(len: usize) -> Vec<f32> {
        clicks(len, 5_760, 14_400, 0.004, true)
    }

    /// Mouse: a sharp 1.5 ms click every 400-700 ms.
    fn mouse(len: usize) -> Vec<f32> {
        clicks(len, 19_200, 33_600, 0.0015, false)
    }

    fn clicks(len: usize, min_gap: usize, max_gap: usize, decay_s: f32, release: bool) -> Vec<f32> {
        let mut seed = 7u32;
        let mut rand = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        let mut out = vec![0.0; len];
        let mut at = 0;
        while at < len {
            let strikes: &[(usize, f32)] = if release { &[(0, 1.0), (3_600, 0.5)] } else { &[(0, 1.0)] };
            for &(offset, strength) in strikes {
                let amp = strength * (0.5 + 0.5 * rand());
                let mut last = 0.0;
                for i in 0..(decay_s * 48_000.0 * 6.0) as usize {
                    let Some(s) = out.get_mut(at + offset + i) else { break };
                    let white = rand() * 2.0 - 1.0;
                    let hp = white - last; // a bit brighter, like a click
                    last = white;
                    *s += amp * hp * (-(i as f32) / (decay_s * 48_000.0)).exp();
                }
            }
            at += min_gap + (rand() * (max_gap - min_gap) as f32) as usize;
        }
        out
    }

    /// A fan: steady low rumble plus some hiss.
    fn fan(len: usize) -> Vec<f32> {
        let mut seed = 3u32;
        let mut low = 0.0;
        (0..len)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let white = (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
                low = 0.98 * low + 0.02 * white;
                low * 4.0 + white * 0.1
            })
            .collect()
    }

    /// Scale-invariant signal-to-distortion ratio (dB) of `estimate` against `clean`: how close
    /// the output is to the voice alone, whatever its volume.
    fn si_sdr(clean: &[f32], estimate: &[f32]) -> f32 {
        let dot: f64 = clean.iter().zip(estimate).map(|(a, b)| *a as f64 * *b as f64).sum();
        let energy: f64 = clean.iter().map(|a| (*a as f64).powi(2)).sum();
        let alpha = dot / energy;
        let (mut target, mut error) = (0.0f64, 0.0f64);
        for (a, b) in clean.iter().zip(estimate) {
            let t = alpha * *a as f64;
            target += t * t;
            error += (*b as f64 - t).powi(2);
        }
        (10.0 * (target / error.max(1e-20)).log10()) as f32
    }

    /// The delay (samples, up to 100 ms) that lines `output` up best with `clean`.
    fn delay_of(clean: &[f32], output: &[f32]) -> usize {
        let n = clean.len().min(output.len()).min(5 * 48_000);
        let corr = |d: usize| -> f64 { (0..n - d).step_by(4).map(|i| clean[i] as f64 * output[i + d] as f64).sum() };
        let best = |lags: &mut dyn Iterator<Item = usize>| lags.map(|d| (d, corr(d))).max_by(|a, b| a.1.total_cmp(&b.1)).unwrap().0;
        let coarse = best(&mut (0..4_800usize).step_by(8)); // first every 8 samples, then around the best
        best(&mut (coarse.saturating_sub(8)..coarse + 8))
    }

    #[test]
    #[ignore = "needs speech: see scripts/make-speech.ps1; run optimized, takes ~1 min"]
    fn denoise_evaluation() {
        let Ok(dir) = std::env::var("JAYA_SPEECH_DIR") else {
            return println!("set JAYA_SPEECH_DIR to a folder made by scripts/make-speech.ps1");
        };
        let dir = std::path::Path::new(&dir);
        let mut speech = read_wav(&dir.join("speech.wav"));
        speech.truncate(12 * 48_000);
        let len = speech.len();
        let babble_voice = read_wav(&dir.join("babble.wav"));
        let babble: Vec<f32> = babble_voice.iter().cycle().take(len).copied().collect();
        let speech_db = level_db(&speech);
        // Measured from 1 s on: before that the models are starting (DeepFilterNet loads for a
        // few hundred ms and lets the audio through meanwhile).
        const SKIP: usize = 48_000;
        // Where the voice is silent (20 ms windows 40 dB under its level): the noise alone.
        let pauses: Vec<usize> =
            (SKIP / 960..len / 960).filter(|w| level_db(&speech[w * 960..(w + 1) * 960]) < speech_db - 40.0).collect();
        let in_pauses = |x: &[f32], d: usize| -> f32 {
            let samples: Vec<f32> = pauses.iter().flat_map(|w| x[(w * 960 + d).min(x.len())..((w + 1) * 960 + d).min(x.len())].iter().copied()).collect();
            level_db(&samples)
        };

        println!("{:<10} {:<14} {:>12} {:>16} {:>9}", "ruído", "filtro", "SI-SDR (dB)", "ruído na pausa", "atraso");
        for (name, noise) in [("teclado", keyboard(len)), ("mouse", mouse(len)), ("ventilador", fan(len)), ("voz atrás", babble.clone())] {
            // Mixed so the noise is 5 dB under the voice.
            let gain = 10f32.powf((speech_db - 5.0 - level_db(&noise)) / 20.0);
            let mix: Vec<f32> = speech.iter().zip(&noise).map(|(s, n)| s + n * gain).collect();
            let mut results: Vec<(&str, Vec<f32>)> = vec![("sem filtro", mix.clone())];
            #[cfg(windows)]
            {
                let mut apm = crate::apm::Apm::new(48_000, 1, 1, false, true, false).unwrap();
                let mut out = mix.clone();
                for frame in out.chunks_exact_mut(FRAME) {
                    apm.process_capture(frame, 0);
                }
                results.push(("WebRTC", out));
            }
            results.push(("RNNoise", run(&mut runner(1.0), &mix, false)));
            results.push(("DeepFilterNet", run(&mut runner(0.0), &mix, true)));
            for (filter, out) in results {
                let d = delay_of(&speech, &out);
                let sdr = si_sdr(&speech[SKIP..len - d], &out[SKIP + d..]);
                let pause = in_pauses(&out, d) - in_pauses(&mix, 0);
                println!("{name:<10} {filter:<14} {sdr:>12.1} {pause:>13.1} dB {:>6.1} ms", d as f32 / 48.0);
            }
        }
    }
}
