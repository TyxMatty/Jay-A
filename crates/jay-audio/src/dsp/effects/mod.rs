//! The effects of stage 2b (docs/ARQUITETURA.md, section 5, "Catálogo de efeitos"). Written from
//! public formulas; ranges and defaults start from EasyEffects' (docs/REFERENCIAS.md, 1.3), whose
//! GPL code is not used.

pub mod biquad;
pub mod compressor;
pub mod deesser;
pub mod denoise;
pub mod eq;
pub mod filter;
pub mod gate;
pub mod limiter;

/// Samples per second, as f32 for time conversions.
const SECOND: f32 = crate::SAMPLE_RATE as f32;

/// One-pole smoothing coefficient for a time constant of `ms` (0 = instant).
fn coef(ms: f32) -> f32 {
    if ms <= 0.0 { 0.0 } else { (-1.0 / (ms / 1000.0 * SECOND)).exp() }
}

/// A (from, to) pair ramped across a frame: the value at sample `i` of `n`.
fn ramp((from, to): (f32, f32), i: usize, n: usize) -> f32 {
    super::lerp(from, to, i, n)
}

#[cfg(test)]
mod tests {
    use std::f32::consts::PI;

    use crate::dsp::FRAME;
    use crate::dsp::chain::{ChainControl, ChainRunner, ChainSpec, SlotSpec};
    use crate::level_db;

    fn sine(freq: f32, amplitude: f32, seconds: f32) -> Vec<f32> {
        let n = (seconds * 48_000.0) as usize / FRAME * FRAME;
        (0..n).map(|i| amplitude * (2.0 * PI * freq * i as f32 / 48_000.0).sin()).collect()
    }

    /// A sine whose RMS level is `db` dBFS.
    fn sine_db(freq: f32, db: f32, seconds: f32) -> Vec<f32> {
        sine(freq, 10f32.powf(db / 20.0) * 2f32.sqrt(), seconds)
    }

    fn runner(kind: &str, params: &[(&str, f32)]) -> ChainRunner {
        let mut slot = SlotSpec::new(format!("{kind}#0"), kind).unwrap();
        for (key, value) in params {
            let i = slot.param_index(key).unwrap_or_else(|| panic!("{kind} has no {key}"));
            slot.values[i] = slot.specs().nth(i).unwrap().clamp(*value);
        }
        ChainControl::new(ChainSpec { slots: vec![slot] }).unwrap().runner()
    }

    fn run(runner: &mut ChainRunner, input: &[f32]) -> Vec<f32> {
        input
            .chunks(FRAME)
            .flat_map(|chunk| {
                let mut frame = chunk.to_vec();
                runner.process(&mut frame);
                frame
            })
            .collect()
    }

    /// Gain (dB) of the effect for a sine at `freq`, measured over the second half (past the
    /// filters' settling).
    fn gain_at(kind: &str, params: &[(&str, f32)], freq: f32) -> f32 {
        let input = sine(freq, 0.25, 0.5);
        let output = run(&mut runner(kind, params), &input);
        let half = input.len() / 2;
        level_db(&output[half..]) - level_db(&input[half..])
    }

    fn near(value: f32, expected: f32, tolerance: f32, what: &str) {
        assert!((value - expected).abs() <= tolerance, "{what}: {value:.2}, expected {expected} ± {tolerance}");
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0, |m, s| m.max(s.abs()))
    }

    // ---------- filter ----------

    #[test]
    fn highpass_80_hz_cuts_the_rumble_and_keeps_the_voice() {
        let g20 = gain_at("filter", &[], 20.0);
        assert!(g20 <= -20.0, "20 Hz through the 80 Hz high-pass: {g20:.1} dB");
        near(gain_at("filter", &[], 1000.0), 0.0, 0.5, "1 kHz");
        let steep = gain_at("filter", &[("slope", 1.0)], 20.0);
        assert!(steep <= g20 - 15.0, "24 dB/oct cuts much more: {steep:.1} vs {g20:.1} dB");
    }

    #[test]
    fn lowpass_bell_notch_and_shelves_shape_as_asked() {
        let lowpass = [("type", 1.0), ("frequency", 3400.0), ("slope", 1.0)];
        assert!(gain_at("filter", &lowpass, 10_000.0) <= -30.0);
        near(gain_at("filter", &lowpass, 1000.0), 0.0, 1.0, "low-pass at 1 kHz");
        let bell = [("type", 4.0), ("frequency", 1000.0), ("q", 1.0), ("gain", 6.0)];
        near(gain_at("filter", &bell, 1000.0), 6.0, 0.3, "bell at its center");
        near(gain_at("filter", &bell, 100.0), 0.0, 0.5, "bell far below");
        let notch = [("type", 5.0), ("frequency", 1000.0), ("q", 2.0)];
        assert!(gain_at("filter", &notch, 1000.0) <= -30.0);
        let low_shelf = [("type", 2.0), ("frequency", 200.0), ("gain", 6.0)];
        near(gain_at("filter", &low_shelf, 40.0), 6.0, 0.5, "low shelf below");
        near(gain_at("filter", &low_shelf, 5000.0), 0.0, 0.5, "low shelf above");
        let high_shelf = [("type", 3.0), ("frequency", 4000.0), ("gain", -6.0)];
        near(gain_at("filter", &high_shelf, 15_000.0), -6.0, 0.7, "high shelf above");
        near(gain_at("filter", &high_shelf, 200.0), 0.0, 0.5, "high shelf below");
    }

    #[test]
    fn moving_a_filter_does_not_click() {
        // A 24 dB/oct high-pass jumping between 20 Hz and 2 kHz every 300 ms while a 1 kHz sine
        // plays: the sine goes from passing to cut by ~20 dB and back.
        let mut slot = SlotSpec::new("filter#0", "filter").unwrap();
        let slope = slot.param_index("slope").unwrap();
        slot.values[slope] = 1.0;
        let mut control = ChainControl::new(ChainSpec { slots: vec![slot] }).unwrap();
        let mut runner = control.runner();
        let input = sine(1000.0, 0.5, 2.0);
        let mut out = Vec::new();
        for (f, chunk) in input.chunks(FRAME).enumerate() {
            control.set_param("filter#0", "frequency", if (f / 30) % 2 == 0 { 2000.0 } else { 20.0 }).unwrap();
            let mut frame = chunk.to_vec();
            runner.process(&mut frame);
            out.extend(frame);
        }
        // The sine alone steps up to 0.5 * 0.131 ≈ 0.065 between samples.
        let step = out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
        assert!(step < 0.08, "a click: neighbouring samples {step} apart");
    }

    // ---------- equalizer ----------

    #[test]
    fn equalizer_is_flat_by_default_and_boosts_a_band() {
        for freq in [50.0, 300.0, 1000.0, 8000.0] {
            near(gain_at("eq", &[], freq), 0.0, 0.05, "flat equalizer");
        }
        let boost = [("band2Gain", 6.0)]; // the 1 kHz bell
        near(gain_at("eq", &boost, 1000.0), 6.0, 0.3, "1 kHz band +6 dB");
        near(gain_at("eq", &boost, 100.0), 0.0, 0.5, "far from the band");
        let off = [("band2Gain", 6.0), ("band2Type", 0.0)];
        near(gain_at("eq", &off, 1000.0), 0.0, 0.05, "a band turned off");
    }

    // ---------- gate ----------

    #[test]
    fn gate_opens_above_the_threshold_and_closes_below_it_minus_the_hysteresis() {
        // Opens at -45 dB, closes below -51 dB, hold 300 ms, -60 dB; release 50 ms (the default
        // 150 ms is a time constant: ~1 s to reach -60 dB, longer than this test).
        let parts = [(-40.0, 0.5), (-48.0, 0.8), (-56.0, 1.2), (-40.0, 0.3)];
        let input: Vec<f32> = parts.iter().flat_map(|&(db, s)| sine_db(1000.0, db, s)).collect();
        let output = run(&mut runner("gate", &[("release", 50.0)]), &input);
        let mut start = 0;
        let mut levels = Vec::new();
        for &(_, seconds) in &parts {
            let end = start + (seconds * 48_000.0) as usize / FRAME * FRAME;
            let tail = end - 9600; // the last 200 ms of each part
            levels.push(level_db(&output[tail..end]) - level_db(&input[tail..end]));
            start = end;
        }
        near(levels[0], 0.0, 0.2, "voice above the threshold passes");
        near(levels[1], 0.0, 0.2, "between the thresholds it stays open");
        assert!(levels[2] <= -55.0, "below the threshold minus the hysteresis it closes: {:.1} dB", levels[2]);
        near(levels[3], 0.0, 0.2, "and opens again");
        let reopen = &output[start - 14_400 + 480..start - 14_400 + 960]; // 10-20 ms after the voice returns
        let source = &input[start - 14_400 + 480..start - 14_400 + 960];
        near(level_db(reopen) - level_db(source), 0.0, 1.0, "opens within the attack");
    }

    // ---------- compressor ----------

    #[test]
    fn compressor_follows_its_curve() {
        // RMS detection: a sine at -9.03 dB RMS, threshold -18, ratio 3 → -18 + 8.97 / 3 = -15.0 dB.
        let input = sine(1000.0, 0.5, 1.0);
        let output = run(&mut runner("compressor", &[]), &input);
        near(level_db(&output[24_000..]), -15.0, 0.3, "compressed RMS level");
        // Peak detection on a square wave (its peak level is constant): -6.02 → -14.0 dBFS.
        let square: Vec<f32> = input.iter().map(|s| 0.5 * s.signum()).collect();
        let output = run(&mut runner("compressor", &[("detection", 1.0)]), &square);
        near(20.0 * peak(&output[24_000..]).log10(), -14.0, 0.3, "compressed peak");
        // Below the threshold nothing changes; makeup adds its gain.
        let quiet = sine(1000.0, 0.05, 0.5);
        let out = run(&mut runner("compressor", &[]), &quiet);
        near(level_db(&out[12_000..]) - level_db(&quiet[12_000..]), 0.0, 0.1, "below the threshold");
        let out = run(&mut runner("compressor", &[("makeup", 6.0)]), &quiet);
        near(level_db(&out[12_000..]) - level_db(&quiet[12_000..]), 6.0, 0.1, "makeup gain");
    }

    #[test]
    fn compressor_curve_matches_the_formula() {
        use super::compressor::curve;
        near(curve(-30.0, -18.0, 3.0, 6.0), -30.0, 1e-4, "below the knee");
        near(curve(-6.0, -18.0, 3.0, 6.0), -14.0, 1e-4, "above the knee");
        near(curve(-18.0, -18.0, 3.0, 6.0), -18.0 - 0.5, 1e-4, "in the knee, (1/3 - 1) * 3² / 12");
        near(curve(-18.0, -18.0, 3.0, 0.0), -18.0, 1e-4, "hard knee at the threshold");
        assert!(curve(-18.0, -18.0, 3.0, 0.0).is_finite());
    }

    // ---------- de-esser ----------

    #[test]
    fn deesser_turns_down_loud_highs_only() {
        // 12 kHz, an octave above the 6 kHz split: almost all of it is in the highs. At -9 dB RMS,
        // threshold -18 and ratio 3 ask for about -5.5 dB.
        let highs = sine(12_000.0, 0.5, 0.5);
        let out = run(&mut runner("deesser", &[]), &highs);
        let cut = level_db(&out[12_000..]) - level_db(&highs[12_000..]);
        assert!(cut <= -4.0, "a loud 12 kHz tone is turned down: {cut:.1} dB");
        let quiet = sine(12_000.0, 0.02, 0.5);
        let out = run(&mut runner("deesser", &[]), &quiet);
        near(level_db(&out[12_000..]) - level_db(&quiet[12_000..]), 0.0, 0.1, "quiet highs untouched");
        let lows = sine(500.0, 0.5, 0.5);
        let out = run(&mut runner("deesser", &[]), &lows);
        near(level_db(&out[12_000..]) - level_db(&lows[12_000..]), 0.0, 0.1, "500 Hz untouched");
    }

    // ---------- limiter ----------

    #[test]
    fn limiter_never_goes_above_the_ceiling() {
        let mut input = sine(1000.0, 2.0, 1.0);
        for i in (0..input.len()).step_by(997) {
            input[i] = if i % 2 == 0 { 4.0 } else { -4.0 }; // isolated spikes
        }
        let mut runner = runner("limiter", &[]);
        assert_eq!(runner.latency(), 240, "5 ms of lookahead");
        let output = run(&mut runner, &input);
        let ceiling = 10f32.powf(-1.0 / 20.0);
        let top = peak(&output);
        assert!(top <= ceiling + 1e-6, "{top} is above the -1 dBFS ceiling ({ceiling})");
        assert!(top > ceiling - 0.01, "and it isn't just turned down: {top}");
    }

    #[test]
    fn limiter_leaves_quiet_audio_alone_only_delayed() {
        let input = sine(1000.0, 0.1, 0.5);
        let output = run(&mut runner("limiter", &[]), &input);
        let worst = (240..input.len()).map(|i| (output[i] - input[i - 240]).abs()).fold(0.0, f32::max);
        assert!(worst < 1e-6, "quiet audio changed by {worst}");
    }

    // ---------- the whole stage ----------

    #[test]
    #[ignore = "measures speed; run optimized: cargo test --release -- --ignored cpu"]
    fn cpu_of_the_whole_chain() {
        // All six effects on, the equalizer's five bands boosting or cutting (so none is skipped).
        let mut slots: Vec<SlotSpec> = ["filter", "gate", "eq", "compressor", "deesser", "limiter"]
            .iter()
            .map(|k| SlotSpec::new(format!("{k}#0"), k).unwrap())
            .collect();
        for band in 0..5 {
            let i = slots[2].param_index(&format!("band{band}Gain")).unwrap();
            slots[2].values[i] = if band % 2 == 0 { 2.0 } else { -2.0 };
        }
        let mut runner = ChainControl::new(ChainSpec { slots }).unwrap().runner();
        let voice: Vec<f32> = sine(220.0, 0.3, 1.0).iter().zip(sine(5500.0, 0.05, 1.0)).map(|(a, b)| a + b).collect();
        let seconds = 60;
        let mut frame = vec![0.0; FRAME];
        let start = std::time::Instant::now();
        for f in 0..seconds * 100 {
            let at = (f % 100) * FRAME;
            frame.copy_from_slice(&voice[at..at + FRAME]);
            runner.process(&mut frame);
            std::hint::black_box(&frame);
        }
        let percent = start.elapsed().as_secs_f64() / seconds as f64 * 100.0;
        println!("the whole chain: {percent:.3}% of one core for {seconds} s of audio");
        assert!(percent < 2.0, "RNF-5 allows 2% of one core: {percent:.3}%");
    }

    #[test]
    fn every_effect_survives_silence_and_full_scale_noise() {
        let mut seed = 1u32;
        let mut noise = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
        };
        let loud: Vec<f32> = (0..48_000).map(|_| noise()).collect();
        for info in ["filter", "eq", "gate", "compressor", "deesser", "limiter"] {
            let mut r = runner(info, &[]);
            let out = run(&mut r, &[vec![0.0; 48_000], loud.clone(), vec![0.0; 48_000]].concat());
            assert!(out.iter().all(|s| s.is_finite()), "{info} produced NaN or infinity");
            assert!(out[out.len() - 480..].iter().all(|s| s.abs() < 1e-3), "{info} rings on after the noise stops");
        }
    }
}
