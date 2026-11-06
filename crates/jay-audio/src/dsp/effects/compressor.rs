//! Compressor: above the threshold, the level grows `ratio` times slower. Feed-forward, with a
//! soft knee (the quadratic knee from the compressor literature, e.g. Giannoulis, Massberg and
//! Reiss, JAES 2012), attack and release on the gain reduction, and makeup gain.

use super::{coef, ramp};
use crate::dsp::effect::{Effect, EffectInfo, NO_LATENCY};
use crate::dsp::params::{ParamSpec, Scale};

const DETECTION: &[&str] = &["rms", "peak"];

static PARAMS: [ParamSpec; 7] = [
    ParamSpec { key: "threshold", min: -60.0, max: 0.0, default: -18.0, step: 0.5, scale: Scale::Db, unit: "dB" },
    ParamSpec { key: "ratio", min: 1.0, max: 20.0, default: 3.0, step: 0.1, scale: Scale::Log, unit: ":1" },
    ParamSpec { key: "knee", min: 0.0, max: 24.0, default: 6.0, step: 0.5, scale: Scale::Linear, unit: "dB" },
    ParamSpec { key: "attack", min: 0.1, max: 200.0, default: 10.0, step: 0.1, scale: Scale::Log, unit: "ms" },
    ParamSpec { key: "release", min: 10.0, max: 2000.0, default: 120.0, step: 1.0, scale: Scale::Log, unit: "ms" },
    ParamSpec { key: "makeup", min: 0.0, max: 24.0, default: 0.0, step: 0.1, scale: Scale::Db, unit: "dB" },
    ParamSpec { key: "detection", min: 0.0, max: 1.0, default: 0.0, step: 1.0, scale: Scale::Choice(DETECTION), unit: "" },
];

pub const INFO: EffectInfo = EffectInfo { kind: "compressor", params: &PARAMS, create: || Box::new(Compressor::new()), latency: NO_LATENCY };

/// The RMS detector's window: ~10 ms.
const RMS_MS: f32 = 10.0;

/// Output level (dB) for an input level (dB): the static curve with a soft knee of width `knee`.
pub fn curve(level: f32, threshold: f32, ratio: f32, knee: f32) -> f32 {
    let over = level - threshold;
    if 2.0 * over <= -knee {
        level
    } else if 2.0 * over.abs() <= knee {
        // knee = 0 never gets here (the branch above takes over <= 0), so no 0 / 0.
        level + (1.0 / ratio - 1.0) * (over + knee / 2.0).powi(2) / (2.0 * knee)
    } else {
        threshold + over / ratio
    }
}

struct Compressor {
    values: [f32; 7],
    attack: f32,
    release: f32,
    rms: f32,
    power: f32,
    /// The smoothed gain reduction, in dB (0 or less).
    reduction: f32,
    makeup: (f32, f32),
}

impl Compressor {
    fn new() -> Compressor {
        let mut c = Compressor { values: PARAMS.map(|p| p.default), attack: 0.0, release: 0.0, rms: coef(RMS_MS), power: 0.0, reduction: 0.0, makeup: (0.0, 0.0) };
        c.update();
        c.reset();
        c
    }

    fn update(&mut self) {
        self.attack = coef(self.values[3]);
        self.release = coef(self.values[4]);
        self.makeup.1 = self.values[5];
    }
}

impl Effect for Compressor {
    fn set_param(&mut self, index: usize, value: f32) {
        self.values[index] = value;
        self.update();
    }

    fn process(&mut self, frame: &mut [f32]) {
        let [threshold, ratio, knee, _, _, _, detection] = self.values;
        let peak = detection == 1.0;
        let n = frame.len();
        for (i, s) in frame.iter_mut().enumerate() {
            let level = if peak {
                20.0 * (s.abs() + 1e-6).log10()
            } else {
                self.power = self.rms * self.power + (1.0 - self.rms) * *s * *s;
                10.0 * (self.power + 1e-12).log10()
            };
            let target = curve(level, threshold, ratio, knee) - level;
            let c = if target < self.reduction { self.attack } else { self.release };
            self.reduction = target + (self.reduction - target) * c;
            *s *= 10f32.powf((self.reduction + ramp(self.makeup, i, n)) / 20.0);
        }
        self.makeup.0 = self.makeup.1;
        if self.power < 1e-30 {
            self.power = 0.0;
        }
    }

    fn reset(&mut self) {
        self.power = 0.0;
        self.reduction = 0.0;
        self.makeup.0 = self.makeup.1;
    }
}
