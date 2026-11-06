//! Lookahead limiter: no sample comes out above the ceiling. The signal is delayed by the
//! lookahead; meanwhile, the gain each incoming sample needs is known, and the gain applied is the
//! minimum of those over a window, then averaged over the same window. A window of lookahead + 1
//! samples guarantees every sample gets at most the gain it needs, already smoothed in over the
//! lookahead instead of jumping. The gain rises back with `release`.

use super::{SECOND, coef};
use crate::dsp::effect::{Effect, EffectInfo};
use crate::dsp::params::{ParamSpec, Scale, db_to_gain};

static PARAMS: [ParamSpec; 3] = [
    ParamSpec { key: "ceiling", min: -24.0, max: 0.0, default: -1.0, step: 0.1, scale: Scale::Db, unit: "dB" },
    ParamSpec { key: "lookahead", min: 1.0, max: 10.0, default: 5.0, step: 0.1, scale: Scale::Linear, unit: "ms" },
    ParamSpec { key: "release", min: 5.0, max: 500.0, default: 50.0, step: 1.0, scale: Scale::Log, unit: "ms" },
];

pub const INFO: EffectInfo = EffectInfo { kind: "limiter", params: &PARAMS, create: || Box::new(Limiter::new()) };

/// The longest lookahead (10 ms); buffers are this big from the start, so a new lookahead never
/// allocates.
const MAX: usize = SECOND as usize / 100;
/// The window holds lookahead + 1 samples.
const CAP: usize = MAX + 1;

struct Limiter {
    ceiling: f32,
    lookahead: usize,
    release: f32,
    delay: Vec<f32>,
    delay_pos: usize,
    // The window's minimum: a monotonic queue (sample number, gain) in a ring.
    queue: Vec<(u64, f32)>,
    queue_head: usize,
    queue_len: usize,
    // The average of the minimums over the window.
    minimums: Vec<f32>,
    minimums_pos: usize,
    sum: f64,
    count: u64,
    gain: f32,
}

impl Limiter {
    fn new() -> Limiter {
        let mut limiter = Limiter {
            ceiling: 0.0,
            lookahead: 0,
            release: 0.0,
            delay: vec![0.0; MAX],
            delay_pos: 0,
            queue: vec![(0, 0.0); CAP],
            queue_head: 0,
            queue_len: 0,
            minimums: vec![0.0; CAP],
            minimums_pos: 0,
            sum: 0.0,
            count: 0,
            gain: 1.0,
        };
        for (i, p) in PARAMS.iter().enumerate() {
            limiter.set_param(i, p.default);
        }
        limiter
    }
}

impl Effect for Limiter {
    fn set_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.ceiling = db_to_gain(value),
            1 => {
                let lookahead = ((value / 1000.0 * SECOND).round() as usize).clamp(1, MAX);
                if lookahead != self.lookahead {
                    // Changes the latency, so the chain is rebuilt around a fresh limiter.
                    self.lookahead = lookahead;
                    self.reset();
                }
            }
            _ => self.release = coef(value),
        }
    }

    fn process(&mut self, frame: &mut [f32]) {
        let window = self.lookahead + 1;
        for s in frame {
            let x = *s;
            let needed = if x.abs() > self.ceiling { self.ceiling / x.abs() } else { 1.0 };

            // Minimum over the last `window` samples.
            if self.queue_len > 0 && self.queue[self.queue_head].0 + window as u64 <= self.count {
                self.queue_head = (self.queue_head + 1) % CAP;
                self.queue_len -= 1;
            }
            while self.queue_len > 0 && self.queue[(self.queue_head + self.queue_len - 1) % CAP].1 >= needed {
                self.queue_len -= 1;
            }
            self.queue[(self.queue_head + self.queue_len) % CAP] = (self.count, needed);
            self.queue_len += 1;
            let minimum = self.queue[self.queue_head].1;

            // Averaged over the window.
            self.sum += (minimum - self.minimums[self.minimums_pos]) as f64;
            self.minimums[self.minimums_pos] = minimum;
            self.minimums_pos = (self.minimums_pos + 1) % window;
            let average = (self.sum / window as f64) as f32;
            self.gain = if average < self.gain { average } else { average + (self.gain - average) * self.release };

            let delayed = std::mem::replace(&mut self.delay[self.delay_pos], x);
            self.delay_pos = (self.delay_pos + 1) % self.lookahead;
            // The clamp only matters right after the ceiling is lowered: the samples already in the
            // window were planned for the old one.
            *s = (delayed * self.gain).clamp(-self.ceiling, self.ceiling);
            self.count += 1;
        }
    }

    fn latency(&self) -> usize {
        self.lookahead
    }

    fn reset(&mut self) {
        let window = self.lookahead + 1;
        self.delay.fill(0.0);
        self.delay_pos = 0;
        self.queue_head = 0;
        self.queue_len = 0;
        self.minimums[..window].fill(1.0);
        self.minimums_pos = 0;
        self.sum = window as f64;
        self.count = 0;
        self.gain = 1.0;
    }
}
