//! De-esser: turns down only the highs ("s", "sh") when they get loud. A Linkwitz-Riley crossover
//! (two Butterworth biquads in series on each side) splits the signal at `frequency`; its two
//! halves are in phase and add back to a flat level, so turning the highs down turns down just
//! the highs. Only the highs go through a compressor.

use super::biquad::{Biquad, Shape};
use super::coef;
use super::compressor::curve;
use crate::dsp::FRAME;
use crate::dsp::effect::{Effect, EffectInfo, NO_LATENCY};
use crate::dsp::params::{ParamSpec, Scale};

static PARAMS: [ParamSpec; 3] = [
    ParamSpec { key: "frequency", min: 3000.0, max: 12000.0, default: 6000.0, step: 10.0, scale: Scale::Log, unit: "Hz" },
    ParamSpec { key: "threshold", min: -60.0, max: 0.0, default: -18.0, step: 0.5, scale: Scale::Db, unit: "dB" },
    ParamSpec { key: "ratio", min: 1.0, max: 20.0, default: 3.0, step: 0.1, scale: Scale::Log, unit: ":1" },
];

pub const INFO: EffectInfo = EffectInfo { kind: "deesser", params: &PARAMS, create: || Box::new(DeEsser::new()), latency: NO_LATENCY };

const ATTACK_MS: f32 = 1.0;
const RELEASE_MS: f32 = 50.0;
const RMS_MS: f32 = 5.0;
/// Butterworth: two in series make a Linkwitz-Riley 24 dB/oct crossover.
const Q: f32 = std::f32::consts::FRAC_1_SQRT_2;

struct DeEsser {
    values: [f32; 3],
    lows: [Biquad; 2],
    highs: [Biquad; 2],
    low: [f32; FRAME],
    high: [f32; FRAME],
    attack: f32,
    release: f32,
    rms: f32,
    power: f32,
    reduction: f32,
}

impl DeEsser {
    fn new() -> DeEsser {
        let mut d = DeEsser {
            values: PARAMS.map(|p| p.default),
            lows: Default::default(),
            highs: Default::default(),
            low: [0.0; FRAME],
            high: [0.0; FRAME],
            attack: coef(ATTACK_MS),
            release: coef(RELEASE_MS),
            rms: coef(RMS_MS),
            power: 0.0,
            reduction: 0.0,
        };
        d.update();
        d.reset();
        d
    }

    fn update(&mut self) {
        let freq = self.values[0];
        for stage in &mut self.lows {
            stage.set(Some(Shape::LowPass), freq, Q, 0.0);
        }
        for stage in &mut self.highs {
            stage.set(Some(Shape::HighPass), freq, Q, 0.0);
        }
    }
}

impl Effect for DeEsser {
    fn set_param(&mut self, index: usize, value: f32) {
        self.values[index] = value;
        if index == 0 {
            self.update();
        }
    }

    fn process(&mut self, frame: &mut [f32]) {
        let [_, threshold, ratio] = self.values;
        let n = frame.len();
        let (low, high) = (&mut self.low[..n], &mut self.high[..n]);
        low.copy_from_slice(frame);
        high.copy_from_slice(frame);
        for stage in &mut self.lows {
            stage.process(low);
        }
        for stage in &mut self.highs {
            stage.process(high);
        }
        for ((s, l), h) in frame.iter_mut().zip(low.iter()).zip(high.iter()) {
            self.power = self.rms * self.power + (1.0 - self.rms) * h * h;
            let level = 10.0 * (self.power + 1e-12).log10();
            let target = curve(level, threshold, ratio, 0.0) - level;
            let c = if target < self.reduction { self.attack } else { self.release };
            self.reduction = target + (self.reduction - target) * c;
            *s = l + h * 10f32.powf(self.reduction / 20.0);
        }
        if self.power < 1e-30 {
            self.power = 0.0;
        }
    }

    fn reset(&mut self) {
        for stage in self.lows.iter_mut().chain(&mut self.highs) {
            stage.reset();
        }
        self.power = 0.0;
        self.reduction = 0.0;
    }
}
