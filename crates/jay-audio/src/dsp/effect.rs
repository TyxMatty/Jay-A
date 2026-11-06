//! What an effect implements, and the list of effects a chain or a preset can name. The host
//! (chain.rs) adds what every effect has: on/off, input and output gain, dry/wet and meters.

use super::params::ParamSpec;

/// Runs on the audio thread: no allocation, no lock, no I/O in any of these.
pub trait Effect: Send {
    /// `index` into the effect's own params; `value` already clamped to its spec. The effect
    /// ramps to it (over a frame at most) instead of jumping, so changes don't click.
    fn set_param(&mut self, index: usize, value: f32);
    /// A frame (`FRAME` samples, mono), in place.
    fn process(&mut self, frame: &mut [f32]);
    /// Samples of delay the effect adds (e.g. a limiter's lookahead). Read when the chain is
    /// built: a parameter that changes it must rebuild the chain.
    fn latency(&self) -> usize {
        0
    }
    /// Forgets the past (envelopes, filter memories) and jumps to the set values. Called when the
    /// chain is built and when the effect is turned back on.
    fn reset(&mut self);
}

pub struct EffectInfo {
    /// The name in presets and slot ids (`gate` in `gate#0`).
    pub kind: &'static str,
    pub params: &'static [ParamSpec],
    pub create: fn() -> Box<dyn Effect>,
    /// The latency the effect will have with these values of its params, without creating it
    /// (creating some, like a neural network, is slow). Must match `Effect::latency`.
    pub latency: fn(&[f32]) -> usize,
}

/// For effects that never add latency.
pub const NO_LATENCY: fn(&[f32]) -> usize = |_| 0;

impl PartialEq for EffectInfo {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
    }
}

impl std::fmt::Debug for EffectInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.kind)
    }
}

/// The effects users can pick, in the order the UI offers them.
static EFFECTS: &[EffectInfo] = {
    use super::effects::{compressor, deesser, denoise, eq, filter, gate, limiter};
    &[denoise::INFO, filter::INFO, gate::INFO, eq::INFO, compressor::INFO, deesser::INFO, limiter::INFO]
};

/// The effects users can pick, in the order the UI offers them.
pub fn all() -> &'static [EffectInfo] {
    EFFECTS
}

pub fn find(kind: &str) -> Option<&'static EffectInfo> {
    EFFECTS.iter().chain(test_effects::EFFECTS).find(|e| e.kind == kind)
}

#[cfg(not(test))]
mod test_effects {
    pub static EFFECTS: &[super::EffectInfo] = &[];
}

/// Small effects that make the chain's behavior easy to check: a gain that ramps, a hard clipper
/// (so the order of effects changes the sound) and a pure delay (latency).
#[cfg(test)]
pub(crate) mod test_effects {
    use super::{Effect, EffectInfo, NO_LATENCY};
    use crate::dsp::lerp;
    use crate::dsp::params::{ParamSpec, Scale, db_to_gain};

    pub static EFFECTS: &[EffectInfo] = &[
        EffectInfo { kind: "test-gain", params: &GAIN, create: || Box::new(Gain { from: 1.0, to: 1.0 }), latency: NO_LATENCY },
        EffectInfo { kind: "test-clip", params: &CLIP, create: || Box::new(Clip(0.3)), latency: NO_LATENCY },
        EffectInfo { kind: "test-delay", params: &DELAY, create: || Box::new(Delay { line: Vec::new(), pos: 0 }), latency: |v| v[0] as usize },
    ];

    static GAIN: [ParamSpec; 1] = [ParamSpec { key: "gain", min: -24.0, max: 24.0, default: 0.0, step: 0.1, scale: Scale::Db, unit: "dB" }];
    static CLIP: [ParamSpec; 1] = [ParamSpec { key: "ceiling", min: 0.01, max: 1.0, default: 0.3, step: 0.01, scale: Scale::Linear, unit: "" }];
    static DELAY: [ParamSpec; 1] = [ParamSpec { key: "samples", min: 0.0, max: 960.0, default: 48.0, step: 1.0, scale: Scale::Linear, unit: "" }];

    struct Gain {
        from: f32,
        to: f32,
    }
    impl Effect for Gain {
        fn set_param(&mut self, _: usize, value: f32) {
            self.to = db_to_gain(value);
        }
        fn process(&mut self, frame: &mut [f32]) {
            let n = frame.len();
            for (i, s) in frame.iter_mut().enumerate() {
                *s *= lerp(self.from, self.to, i, n);
            }
            self.from = self.to;
        }
        fn reset(&mut self) {
            self.from = self.to;
        }
    }

    struct Clip(f32);
    impl Effect for Clip {
        fn set_param(&mut self, _: usize, value: f32) {
            self.0 = value;
        }
        fn process(&mut self, frame: &mut [f32]) {
            for s in frame {
                *s = s.clamp(-self.0, self.0);
            }
        }
        fn reset(&mut self) {}
    }

    /// The delay is set before the chain is built (latency is read then), so resizing here is
    /// off the audio thread.
    struct Delay {
        line: Vec<f32>,
        pos: usize,
    }
    impl Effect for Delay {
        fn set_param(&mut self, _: usize, value: f32) {
            self.line = vec![0.0; value as usize];
            self.pos = 0;
        }
        fn process(&mut self, frame: &mut [f32]) {
            if self.line.is_empty() {
                return;
            }
            for s in frame {
                std::mem::swap(s, &mut self.line[self.pos]);
                self.pos = (self.pos + 1) % self.line.len();
            }
        }
        fn latency(&self) -> usize {
            self.line.len()
        }
        fn reset(&mut self) {
            self.line.fill(0.0);
            self.pos = 0;
        }
    }
}
