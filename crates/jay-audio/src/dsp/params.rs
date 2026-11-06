//! What a parameter accepts: the UI draws its control from this, and every value that comes from
//! the UI or a preset is made valid by `clamp` (the idea of Carla's ParameterRanges,
//! docs/REFERENCIAS.md 2.3).

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scale {
    Linear,
    /// Frequencies and times: the control moves on a log scale.
    Log,
    Db,
    /// 0 or 1.
    Toggle,
    /// An index into the labels (presets store the label).
    Choice(&'static [&'static str]),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamSpec {
    pub key: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub step: f32,
    pub scale: Scale,
    pub unit: &'static str,
}

impl ParamSpec {
    /// `value` inside the range (whole numbers for toggles and choices); NaN becomes the default.
    pub fn clamp(&self, value: f32) -> f32 {
        if value.is_nan() {
            return self.default;
        }
        let value = value.clamp(self.min, self.max);
        match self.scale {
            Scale::Toggle | Scale::Choice(_) => value.round(),
            _ => value,
        }
    }
}

/// The controls every effect gets from the host (EasyEffects' input/output gain and Carla's
/// dry/wet, docs/REFERENCIAS.md): a slot's values start with these, then the effect's own.
pub const COMMON: [ParamSpec; 4] = [
    ParamSpec { key: "enabled", min: 0.0, max: 1.0, default: 1.0, step: 1.0, scale: Scale::Toggle, unit: "" },
    ParamSpec { key: "inputGain", min: -24.0, max: 24.0, default: 0.0, step: 0.1, scale: Scale::Db, unit: "dB" },
    ParamSpec { key: "outputGain", min: -24.0, max: 24.0, default: 0.0, step: 0.1, scale: Scale::Db, unit: "dB" },
    ParamSpec { key: "mix", min: 0.0, max: 100.0, default: 100.0, step: 1.0, scale: Scale::Linear, unit: "%" },
];
pub const ENABLED: usize = 0;
pub const INPUT_GAIN: usize = 1;
pub const OUTPUT_GAIN: usize = 2;
pub const MIX: usize = 3;

pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_keeps_values_valid() {
        let gain = COMMON[INPUT_GAIN];
        assert_eq!(gain.clamp(-100.0), -24.0);
        assert_eq!(gain.clamp(100.0), 24.0);
        assert_eq!(gain.clamp(3.5), 3.5);
        assert_eq!(gain.clamp(f32::NAN), 0.0, "NaN becomes the default");
        assert_eq!(gain.clamp(f32::INFINITY), 24.0);
        assert_eq!(gain.clamp(f32::NEG_INFINITY), -24.0);
        assert_eq!(COMMON[ENABLED].clamp(0.7), 1.0, "toggles are 0 or 1");
        let choice = ParamSpec { key: "type", min: 0.0, max: 2.0, default: 0.0, step: 1.0, scale: Scale::Choice(&["a", "b", "c"]), unit: "" };
        assert_eq!(choice.clamp(1.4), 1.0);
        assert_eq!(choice.clamp(9.0), 2.0);
    }
}
