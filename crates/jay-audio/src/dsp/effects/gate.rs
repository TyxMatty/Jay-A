//! Noise gate: lets the voice through and turns everything quieter down. It opens above `open`
//! and only closes below `open - hysteresis` (so a voice near the threshold doesn't make it
//! flutter), and stays open `hold` ms after that, so word endings aren't cut (Jay-C's mic gate
//! has the same hold).

use super::{SECOND, coef};
use crate::dsp::effect::{Effect, EffectInfo, NO_LATENCY};
use crate::dsp::params::{ParamSpec, Scale, db_to_gain};

static PARAMS: [ParamSpec; 6] = [
    ParamSpec { key: "open", min: -80.0, max: 0.0, default: -45.0, step: 0.5, scale: Scale::Db, unit: "dB" },
    ParamSpec { key: "hysteresis", min: 0.0, max: 20.0, default: 6.0, step: 0.5, scale: Scale::Linear, unit: "dB" },
    ParamSpec { key: "attack", min: 0.0, max: 50.0, default: 2.0, step: 0.1, scale: Scale::Linear, unit: "ms" },
    ParamSpec { key: "hold", min: 0.0, max: 1000.0, default: 300.0, step: 10.0, scale: Scale::Linear, unit: "ms" },
    ParamSpec { key: "release", min: 5.0, max: 2000.0, default: 150.0, step: 1.0, scale: Scale::Log, unit: "ms" },
    ParamSpec { key: "reduction", min: -80.0, max: 0.0, default: -60.0, step: 0.5, scale: Scale::Db, unit: "dB" },
];

pub const INFO: EffectInfo = EffectInfo { kind: "gate", params: &PARAMS, create: || Box::new(Gate::new()), latency: NO_LATENCY };

/// The level detector: mean square over ~5 ms.
const DETECT_MS: f32 = 5.0;

struct Gate {
    values: [f32; 6],
    open_power: f32,
    close_power: f32,
    attack: f32,
    hold: usize,
    release: f32,
    closed_gain: f32,
    detect: f32,
    power: f32,
    is_open: bool,
    hold_left: usize,
    gain: f32,
}

impl Gate {
    fn new() -> Gate {
        let mut gate = Gate {
            values: PARAMS.map(|p| p.default),
            open_power: 0.0,
            close_power: 0.0,
            attack: 0.0,
            hold: 0,
            release: 0.0,
            closed_gain: 0.0,
            detect: coef(DETECT_MS),
            power: 0.0,
            is_open: false,
            hold_left: 0,
            gain: 0.0,
        };
        gate.update();
        gate.reset();
        gate
    }

    fn update(&mut self) {
        let [open, hysteresis, attack, hold, release, reduction] = self.values;
        // Compared as power (mean square), so no log per sample.
        self.open_power = 10f32.powf(open / 10.0);
        self.close_power = 10f32.powf((open - hysteresis) / 10.0);
        self.attack = coef(attack);
        self.hold = (hold / 1000.0 * SECOND) as usize;
        self.release = coef(release);
        self.closed_gain = db_to_gain(reduction);
    }
}

impl Effect for Gate {
    fn set_param(&mut self, index: usize, value: f32) {
        self.values[index] = value;
        self.update();
    }

    fn process(&mut self, frame: &mut [f32]) {
        for s in frame {
            self.power = self.detect * self.power + (1.0 - self.detect) * *s * *s;
            if self.power >= self.open_power {
                self.is_open = true;
                self.hold_left = self.hold;
            } else if self.power < self.close_power {
                if self.hold_left > 0 {
                    self.hold_left -= 1;
                } else {
                    self.is_open = false;
                }
            } else if self.is_open {
                self.hold_left = self.hold; // between the two thresholds an open gate stays open
            }
            let target = if self.is_open { 1.0 } else { self.closed_gain };
            let c = if target > self.gain { self.attack } else { self.release };
            self.gain = target + (self.gain - target) * c;
            *s *= self.gain;
        }
        if self.power < 1e-30 {
            self.power = 0.0; // no denormals in long silences
        }
    }

    fn reset(&mut self) {
        self.power = 0.0;
        self.is_open = false;
        self.hold_left = 0;
        self.gain = self.closed_gain;
    }
}
