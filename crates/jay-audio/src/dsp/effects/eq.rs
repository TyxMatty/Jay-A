//! Parametric equalizer: up to 10 bands, each one a filter (bell, shelf, pass or notch) or off.
//! By default 5 flat bells (100, 300, 1k, 3k, 8k Hz) and 5 bands off.

use super::biquad::{Biquad, Shape};
use crate::dsp::effect::{Effect, EffectInfo, NO_LATENCY};
use crate::dsp::params::{ParamSpec, Scale};

pub const BANDS: usize = 10;
const TYPES: &[&str] = &["off", "bell", "lowshelf", "highshelf", "highpass", "lowpass", "notch"];
const SHAPES: [Option<Shape>; 7] =
    [None, Some(Shape::Bell), Some(Shape::LowShelf), Some(Shape::HighShelf), Some(Shape::HighPass), Some(Shape::LowPass), Some(Shape::Notch)];

/// Four parameters per band: `band<N>Type`, `band<N>Frequency`, `band<N>Gain`, `band<N>Q`.
macro_rules! bands {
    ($($n:literal $kind:literal $freq:literal),*) => {
        [$(
            ParamSpec { key: concat!("band", $n, "Type"), min: 0.0, max: 6.0, default: $kind, step: 1.0, scale: Scale::Choice(TYPES), unit: "" },
            ParamSpec { key: concat!("band", $n, "Frequency"), min: 20.0, max: 20000.0, default: $freq, step: 1.0, scale: Scale::Log, unit: "Hz" },
            ParamSpec { key: concat!("band", $n, "Gain"), min: -24.0, max: 24.0, default: 0.0, step: 0.1, scale: Scale::Db, unit: "dB" },
            ParamSpec { key: concat!("band", $n, "Q"), min: 0.1, max: 10.0, default: 1.0, step: 0.01, scale: Scale::Log, unit: "" },
        )*]
    };
}

static PARAMS: [ParamSpec; BANDS * 4] = bands!(
    0 1.0 100.0, 1 1.0 300.0, 2 1.0 1000.0, 3 1.0 3000.0, 4 1.0 8000.0,
    5 0.0 1000.0, 6 0.0 1000.0, 7 0.0 1000.0, 8 0.0 1000.0, 9 0.0 1000.0
);

pub const INFO: EffectInfo = EffectInfo { kind: "eq", params: &PARAMS, create: || Box::new(Equalizer::new()), latency: NO_LATENCY };

struct Equalizer {
    values: [f32; BANDS * 4],
    bands: [Biquad; BANDS],
}

impl Equalizer {
    fn new() -> Equalizer {
        let mut eq = Equalizer { values: PARAMS.map(|p| p.default), bands: Default::default() };
        for band in 0..BANDS {
            eq.update(band);
        }
        eq.reset();
        eq
    }

    fn update(&mut self, band: usize) {
        let [kind, freq, gain, q] = [0, 1, 2, 3].map(|k| self.values[band * 4 + k]);
        self.bands[band].set(SHAPES[kind as usize], freq, q, gain);
    }
}

impl Effect for Equalizer {
    fn set_param(&mut self, index: usize, value: f32) {
        self.values[index] = value;
        self.update(index / 4);
    }

    fn process(&mut self, frame: &mut [f32]) {
        for band in &mut self.bands {
            band.process(frame);
        }
    }

    fn reset(&mut self) {
        for band in &mut self.bands {
            band.reset();
        }
    }
}
