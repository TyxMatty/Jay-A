//! Presets: the chain's order and each effect's values in JSON, like EasyEffects' presets
//! (docs/ARQUITETURA.md, section 5). A preset file is untrusted input: reading one never fails on
//! a bad effect or value, it skips or fixes it and says so; only a file that isn't a Jay-A preset
//! (or is from a newer Jay-A) is refused.

use serde_json::{Map, Value, json};

use super::chain::{ChainSpec, MAX_SLOTS, SlotSpec};
use super::params::Scale;

pub const FORMAT: &str = "jay-a-preset";
pub const VERSION: u64 = 1;
/// Longer ids are refused (they end up in the UI and in file names of exports).
const MAX_ID: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub name: String,
    pub chain: ChainSpec,
    /// The WebRTC processing in front of the chain; None = leave it as it is.
    pub noise_suppression: Option<bool>,
    pub auto_gain_control: Option<bool>,
}

/// Reads a preset. Returns it with the warnings about what was skipped or fixed.
pub fn parse(text: &str) -> Result<(Preset, Vec<String>), String> {
    let root: Value = serde_json::from_str(text).map_err(|e| format!("not a preset: {e}"))?;
    if root.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err("not a Jay-A preset".into());
    }
    match root.get("version").and_then(Value::as_u64) {
        Some(v) if (1..=VERSION).contains(&v) => {}
        Some(v) => return Err(format!("this preset is version {v}; update Jay-A to open it")),
        None => return Err("the preset has no version".into()),
    }

    let mut warnings = Vec::new();
    let empty = Map::new();
    let effects = root.get("effects").and_then(Value::as_object).unwrap_or(&empty);
    let mut chain = ChainSpec::default();
    for id in root.get("chain").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default() {
        let Some(id) = id.as_str().filter(|id| !id.is_empty() && id.len() <= MAX_ID) else {
            warnings.push(format!("skipped an effect with an invalid name: {id}"));
            continue;
        };
        if chain.slots.len() == MAX_SLOTS {
            warnings.push(format!("only the first {MAX_SLOTS} effects were loaded"));
            break;
        }
        if chain.slots.iter().any(|s| s.id == id) {
            warnings.push(format!("{id} appears twice; the second was skipped"));
            continue;
        }
        let kind = id.split('#').next().unwrap_or(id);
        let Some(mut slot) = SlotSpec::new(id, kind) else {
            warnings.push(format!("unknown effect {kind}, skipped"));
            continue;
        };
        for (key, value) in effects.get(id).and_then(Value::as_object).unwrap_or(&empty) {
            let Some(index) = slot.param_index(key) else {
                warnings.push(format!("{id}: unknown parameter {key}, ignored"));
                continue;
            };
            let spec = slot.specs().nth(index).expect("index from param_index");
            let number = match (value, spec.scale) {
                (Value::Bool(b), _) => Some(if *b { 1.0 } else { 0.0 }),
                (Value::String(label), Scale::Choice(labels)) => labels.iter().position(|l| l == label).map(|i| i as f32),
                (Value::Number(n), _) => n.as_f64().map(|n| n as f32),
                _ => None,
            };
            let Some(number) = number else {
                warnings.push(format!("{id}: invalid value for {key}, using the default"));
                continue;
            };
            let clamped = spec.clamp(number);
            if clamped != number {
                warnings.push(format!("{id}: {key} = {number} is out of range, set to {clamped}"));
            }
            slot.values[index] = clamped;
        }
        chain.slots.push(slot);
    }

    let webrtc = root.get("webrtc");
    let flag = |key: &str| webrtc.and_then(|w| w.get(key)).and_then(Value::as_bool);
    let preset = Preset {
        name: root.get("name").and_then(Value::as_str).unwrap_or_default().chars().take(MAX_ID).collect(),
        chain,
        noise_suppression: flag("noiseSuppression"),
        auto_gain_control: flag("autoGainControl"),
    };
    Ok((preset, warnings))
}

/// Writes a preset: toggles as booleans, choices as their labels, numbers rounded to 4 decimals.
pub fn to_json(preset: &Preset) -> String {
    let mut effects = Map::new();
    for slot in &preset.chain.slots {
        let values: Map<String, Value> = slot
            .specs()
            .zip(&slot.values)
            .map(|(spec, &v)| {
                let value = match spec.scale {
                    Scale::Toggle => json!(v != 0.0),
                    Scale::Choice(labels) => json!(labels[v as usize]),
                    _ => json!((v as f64 * 1e4).round() / 1e4),
                };
                (spec.key.to_string(), value)
            })
            .collect();
        effects.insert(slot.id.clone(), Value::Object(values));
    }
    let mut webrtc = Map::new();
    if let Some(on) = preset.noise_suppression {
        webrtc.insert("noiseSuppression".into(), json!(on));
    }
    if let Some(on) = preset.auto_gain_control {
        webrtc.insert("autoGainControl".into(), json!(on));
    }
    let root = json!({
        "format": FORMAT,
        "version": VERSION,
        "name": preset.name,
        "chain": preset.chain.slots.iter().map(|s| &s.id).collect::<Vec<_>>(),
        "effects": effects,
        "webrtc": webrtc,
    });
    serde_json::to_string_pretty(&root).expect("a JSON value always serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::params::{COMMON, ENABLED, INPUT_GAIN, MIX};

    #[test]
    fn reads_a_preset_and_says_what_it_skipped_or_fixed() {
        let text = r#"{
            "format": "jay-a-preset", "version": 1, "name": "Teste",
            "chain": ["test-gain#0", "reverb#0", "test-clip#0", "test-gain#0", 7],
            "effects": {
                "test-gain#0": { "enabled": false, "inputGain": 99, "gain": -6, "color": "blue" },
                "test-clip#0": { "mix": "lots", "ceiling": 0.5 }
            },
            "webrtc": { "noiseSuppression": false }
        }"#;
        let (preset, warnings) = parse(text).unwrap();
        assert_eq!(preset.name, "Teste");
        let ids: Vec<&str> = preset.chain.slots.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["test-gain#0", "test-clip#0"]);
        let gain = &preset.chain.slots[0].values;
        assert_eq!(gain[ENABLED], 0.0);
        assert_eq!(gain[INPUT_GAIN], 24.0, "clamped");
        assert_eq!(gain[COMMON.len()], -6.0);
        let clip = &preset.chain.slots[1].values;
        assert_eq!(clip[MIX], 100.0, "an invalid value keeps the default");
        assert_eq!(clip[COMMON.len()], 0.5);
        assert_eq!(preset.noise_suppression, Some(false));
        assert_eq!(preset.auto_gain_control, None);
        for expected in ["reverb", "appears twice", "invalid name", "out of range", "color", "invalid value for mix"] {
            assert!(warnings.iter().any(|w| w.contains(expected)), "no warning about {expected}: {warnings:?}");
        }
        assert_eq!(warnings.len(), 6, "{warnings:?}");
    }

    #[test]
    fn refuses_what_is_not_a_jay_a_preset() {
        assert!(parse("not json").is_err());
        assert!(parse(r#"{"format": "easyeffects", "version": 1}"#).is_err());
        assert!(parse(r#"{"format": "jay-a-preset"}"#).is_err(), "no version");
        let newer = parse(r#"{"format": "jay-a-preset", "version": 2, "chain": []}"#).unwrap_err();
        assert!(newer.contains("update Jay-A"), "{newer}");
    }

    #[test]
    fn a_saved_preset_reads_back_the_same() {
        let mut chain = ChainSpec { slots: vec![SlotSpec::new("test-gain#0", "test-gain").unwrap(), SlotSpec::new("test-clip#3", "test-clip").unwrap()] };
        chain.slots[0].values[INPUT_GAIN] = -3.5;
        chain.slots[1].values[ENABLED] = 0.0;
        chain.slots[1].values[COMMON.len()] = 0.25;
        let preset = Preset { name: "Voz".into(), chain, noise_suppression: Some(true), auto_gain_control: Some(false) };
        let (read, warnings) = parse(&to_json(&preset)).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(read, preset);
    }

    #[test]
    fn a_preset_cannot_make_a_chain_too_long() {
        let ids: Vec<String> = (0..100).map(|i| format!("\"test-gain#{i}\"")).collect();
        let text = format!(r#"{{"format": "jay-a-preset", "version": 1, "chain": [{}]}}"#, ids.join(","));
        let (preset, warnings) = parse(&text).unwrap();
        assert_eq!(preset.chain.slots.len(), MAX_SLOTS);
        assert!(warnings.iter().any(|w| w.contains("first")));
    }
}
