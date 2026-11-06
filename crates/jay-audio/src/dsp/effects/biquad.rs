//! Second-order filters from Robert Bristow-Johnson's public "Audio EQ Cookbook" formulas, run in
//! transposed direct form II in f64 (f32 loses precision on low frequencies at 48 kHz).
//!
//! Changing a filter doesn't click: its settings move toward the new ones at a limited speed (half
//! an octave, half an octave of Q and 2 dB per 10 ms frame), and within each frame the
//! coefficients glide sample by sample. A big jump (typing 2000 Hz over 20 Hz) becomes a ~130 ms
//! sweep; gliding only the coefficients across one frame still clicked on jumps like that.

use crate::SAMPLE_RATE;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    HighPass,
    LowPass,
    LowShelf,
    HighShelf,
    Bell,
    Notch,
}

/// b0, b1, b2, a1, a2 (divided by a0).
pub type Coefs = [f64; 5];
/// Passes the signal untouched.
pub const IDENTITY: Coefs = [1.0, 0.0, 0.0, 0.0, 0.0];

/// `gain_db` only matters for shelves and bells.
pub fn design(shape: Shape, freq: f32, q: f32, gain_db: f32) -> Coefs {
    let fs = SAMPLE_RATE as f64;
    let w0 = 2.0 * std::f64::consts::PI * (freq as f64).clamp(10.0, fs * 0.45) / fs;
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / (2.0 * (q as f64).max(0.05));
    let a = 10f64.powf(gain_db as f64 / 40.0);
    let sa = 2.0 * a.sqrt() * alpha;
    let [b0, b1, b2, a0, a1, a2] = match shape {
        Shape::LowPass => [(1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha],
        Shape::HighPass => [(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha],
        Shape::Bell => [1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a],
        Shape::Notch => [1.0, -2.0 * cos, 1.0, 1.0 + alpha, -2.0 * cos, 1.0 - alpha],
        Shape::LowShelf => [
            a * ((a + 1.0) - (a - 1.0) * cos + sa),
            2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
            a * ((a + 1.0) - (a - 1.0) * cos - sa),
            (a + 1.0) + (a - 1.0) * cos + sa,
            -2.0 * ((a - 1.0) + (a + 1.0) * cos),
            (a + 1.0) + (a - 1.0) * cos - sa,
        ],
        Shape::HighShelf => [
            a * ((a + 1.0) + (a - 1.0) * cos + sa),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
            a * ((a + 1.0) + (a - 1.0) * cos - sa),
            (a + 1.0) - (a - 1.0) * cos + sa,
            2.0 * ((a - 1.0) - (a + 1.0) * cos),
            (a + 1.0) - (a - 1.0) * cos - sa,
        ],
    };
    [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0]
}

/// The most the settings move per frame: log2 frequency, log2 Q, gain in dB.
const STEP: [f32; 3] = [0.5, 0.5, 2.0];

#[derive(Debug, Clone)]
pub struct Biquad {
    /// None = pass-through.
    shape: Option<Shape>,
    /// log2 frequency, log2 Q, gain dB: where the filter is now, and where it's going.
    now: [f32; 3],
    want: [f32; 3],
    shape_changed: bool,
    /// The coefficients the last frame ended with.
    coefs: Coefs,
    z1: f64,
    z2: f64,
}

impl Default for Biquad {
    /// A pass-through.
    fn default() -> Biquad {
        Biquad { shape: None, now: [0.0; 3], want: [0.0; 3], shape_changed: false, coefs: IDENTITY, z1: 0.0, z2: 0.0 }
    }
}

impl Biquad {
    /// New settings, reached at the limited speed. `gain_db` only matters for shelves and bells.
    pub fn set(&mut self, shape: Option<Shape>, freq: f32, q: f32, gain_db: f32) {
        self.shape_changed |= shape != self.shape;
        self.shape = shape;
        self.want = [freq.max(1.0).log2(), q.max(0.01).log2(), gain_db];
    }

    /// Forgets the past and jumps to the settings.
    pub fn reset(&mut self) {
        self.now = self.want;
        self.coefs = self.design();
        self.shape_changed = false;
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    fn design(&self) -> Coefs {
        let [freq, q, gain] = self.now;
        match self.shape {
            None => IDENTITY,
            Some(Shape::Bell) if gain == 0.0 => IDENTITY, // a flat bell: skip it (cheaper, and exact)
            Some(shape) => design(shape, freq.exp2(), q.exp2(), gain),
        }
    }

    pub fn process(&mut self, frame: &mut [f32]) {
        let moving = self.now != self.want;
        if moving {
            for ((now, want), step) in self.now.iter_mut().zip(self.want).zip(STEP) {
                *now += (want - *now).clamp(-step, step);
            }
        }
        let (from, to) = (self.coefs, if moving || self.shape_changed { self.design() } else { self.coefs });
        self.shape_changed = false;
        self.coefs = to;
        if from == IDENTITY && to == IDENTITY {
            self.z1 = 0.0; // what's left of the last frame before it became a pass-through
            self.z2 = 0.0;
            return;
        }
        let n = frame.len() as f64;
        let mut c = to;
        for (i, s) in frame.iter_mut().enumerate() {
            if from != to {
                let t = (i + 1) as f64 / n;
                for k in 0..5 {
                    c[k] = from[k] + (to[k] - from[k]) * t;
                }
            }
            let x = *s as f64;
            let y = c[0] * x + self.z1;
            self.z1 = c[1] * x - c[3] * y + self.z2;
            self.z2 = c[2] * x - c[4] * y;
            *s = y as f32;
        }
        // In silence the state decays into denormals, which are very slow on x86.
        if self.z1.abs() < 1e-20 {
            self.z1 = 0.0;
        }
        if self.z2.abs() < 1e-20 {
            self.z2 = 0.0;
        }
    }
}
