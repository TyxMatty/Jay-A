//! One filter: high-pass, low-pass, shelves, bell or notch. High- and low-pass can be 12 or
//! 24 dB/octave (two stages in series).

use super::biquad::{Biquad, Shape};
use crate::dsp::effect::{Effect, EffectInfo, NO_LATENCY};
use crate::dsp::params::{ParamSpec, Scale};

pub const TYPES: &[&str] = &["highpass", "lowpass", "lowshelf", "highshelf", "bell", "notch"];
const SHAPES: [Shape; 6] = [Shape::HighPass, Shape::LowPass, Shape::LowShelf, Shape::HighShelf, Shape::Bell, Shape::Notch];
const SLOPES: &[&str] = &["12", "24"];

static PARAMS: [ParamSpec; 5] = [
    ParamSpec { key: "type", min: 0.0, max: 5.0, default: 0.0, step: 1.0, scale: Scale::Choice(TYPES), unit: "" },
    ParamSpec { key: "frequency", min: 20.0, max: 20000.0, default: 80.0, step: 1.0, scale: Scale::Log, unit: "Hz" },
    ParamSpec { key: "q", min: 0.1, max: 10.0, default: 0.71, step: 0.01, scale: Scale::Log, unit: "" },
    ParamSpec { key: "gain", min: -24.0, max: 24.0, default: 0.0, step: 0.1, scale: Scale::Db, unit: "dB" },
    ParamSpec { key: "slope", min: 0.0, max: 1.0, default: 0.0, step: 1.0, scale: Scale::Choice(SLOPES), unit: "dB/oct" },
];

pub const INFO: EffectInfo = EffectInfo { kind: "filter", params: &PARAMS, create: || Box::new(Filter::new()), latency: NO_LATENCY };

struct Filter {
    values: [f32; 5],
    stages: [Biquad; 2],
}

impl Filter {
    fn new() -> Filter {
        let mut filter = Filter { values: PARAMS.map(|p| p.default), stages: Default::default() };
        filter.update();
        filter.reset();
        filter
    }

    fn update(&mut self) {
        let [kind, freq, q, gain, slope] = self.values;
        let shape = SHAPES[kind as usize];
        self.stages[0].set(Some(shape), freq, q, gain);
        let steep = slope == 1.0 && matches!(shape, Shape::HighPass | Shape::LowPass);
        // The second stage glides from and to a pass-through, so changing the slope doesn't click.
        self.stages[1].set(steep.then_some(shape), freq, q, gain);
    }
}

impl Effect for Filter {
    fn set_param(&mut self, index: usize, value: f32) {
        self.values[index] = value;
        self.update();
    }

    fn process(&mut self, frame: &mut [f32]) {
        for stage in &mut self.stages {
            stage.process(frame);
        }
    }

    fn reset(&mut self) {
        for stage in &mut self.stages {
            stage.reset();
        }
    }
}
