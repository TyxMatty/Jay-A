//! The rack: the user's effects in their order (docs/ARQUITETURA.md, section 5).
//!
//! Two halves. `ChainControl` lives off the audio thread: it holds what the chain should be
//! (`ChainSpec`), validates changes and builds new chains. `ChainRunner` lives on the audio thread
//! and only ever takes messages with `try_recv`, never waits, allocates or frees: a reorder arrives
//! as a whole chain built elsewhere, is crossfaded in over one frame, and the old chain goes back
//! to be freed elsewhere (Carla's rack skips a plugin it can't lock; here nothing is locked).

use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

use super::effect::{Effect, EffectInfo, find};
use super::params::{COMMON, ENABLED, INPUT_GAIN, MIX, OUTPUT_GAIN, ParamSpec, db_to_gain};
use super::{FRAME, lerp};

/// More than this many effects in one chain is refused (presets are untrusted input).
pub const MAX_SLOTS: usize = 32;
/// Messages waiting for the audio thread; it takes them every 10 ms.
const QUEUE: usize = 256;
/// Old chains waiting to be freed off the audio thread.
const RETIRED: usize = 4;

/// One effect in the chain as the user set it: `values` are `COMMON`'s, then the effect's own,
/// all within their specs.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotSpec {
    /// Unique in the chain, like EasyEffects' `gate#0`.
    pub id: String,
    pub info: &'static EffectInfo,
    pub values: Vec<f32>,
}

impl SlotSpec {
    /// The effect `kind` with every value at its default; None if there's no such effect.
    pub fn new(id: impl Into<String>, kind: &str) -> Option<SlotSpec> {
        let info = find(kind)?;
        Some(SlotSpec { id: id.into(), info, values: COMMON.iter().chain(info.params).map(|p| p.default).collect() })
    }

    /// The specs of `values`, in the same order (all static: the iterator doesn't borrow the slot).
    pub fn specs(&self) -> impl Iterator<Item = &'static ParamSpec> + use<> {
        COMMON.iter().chain(self.info.params)
    }

    pub fn param_index(&self, key: &str) -> Option<usize> {
        self.specs().position(|p| p.key == key)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChainSpec {
    pub slots: Vec<SlotSpec>,
}

impl ChainSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.slots.len() > MAX_SLOTS {
            return Err(format!("a chain holds at most {MAX_SLOTS} effects"));
        }
        for (i, slot) in self.slots.iter().enumerate() {
            if self.slots[..i].iter().any(|s| s.id == slot.id) {
                return Err(format!("two effects named {}", slot.id));
            }
            if slot.values.len() != slot.specs().count() {
                return Err(format!("{}: wrong number of values", slot.id));
            }
        }
        Ok(())
    }

    /// Samples of delay the chain adds (off the audio thread: creates its effects).
    pub fn latency(&self) -> usize {
        self.slots.iter().map(latency_of).sum()
    }

    fn position(&self, id: &str) -> Result<usize, String> {
        self.slots.iter().position(|s| s.id == id).ok_or_else(|| format!("no effect {id}"))
    }

    /// `kind#N` with the lowest N not in use.
    fn free_id(&self, kind: &str) -> String {
        (0..).map(|n| format!("{kind}#{n}")).find(|id| self.slots.iter().all(|s| &s.id != id)).expect("ids are endless")
    }

    /// Adds the effect `kind` at the end, with its defaults. Returns its id.
    pub fn add(&mut self, kind: &str) -> Result<String, String> {
        if self.slots.len() == MAX_SLOTS {
            return Err(format!("a chain holds at most {MAX_SLOTS} effects"));
        }
        let slot = SlotSpec::new(self.free_id(kind), kind).ok_or_else(|| format!("no effect {kind}"))?;
        let id = slot.id.clone();
        self.slots.push(slot);
        Ok(id)
    }

    /// Adds a copy of `id` right after it. Returns the copy's id.
    pub fn duplicate(&mut self, id: &str) -> Result<String, String> {
        if self.slots.len() == MAX_SLOTS {
            return Err(format!("a chain holds at most {MAX_SLOTS} effects"));
        }
        let at = self.position(id)?;
        let mut copy = self.slots[at].clone();
        copy.id = self.free_id(copy.info.kind);
        let new_id = copy.id.clone();
        self.slots.insert(at + 1, copy);
        Ok(new_id)
    }

    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        let at = self.position(id)?;
        self.slots.remove(at);
        Ok(())
    }

    /// Moves `id` to position `to` (0 = first; past the end = last).
    pub fn move_to(&mut self, id: &str, to: usize) -> Result<(), String> {
        let at = self.position(id)?;
        let slot = self.slots.remove(at);
        self.slots.insert(to.min(self.slots.len()), slot);
        Ok(())
    }
}

/// A value that ramps to its target over one frame.
#[derive(Clone, Copy)]
struct Ramp {
    from: f32,
    to: f32,
}

impl Ramp {
    fn new(value: f32) -> Ramp {
        Ramp { from: value, to: value }
    }
    fn at(&self, i: usize, n: usize) -> f32 {
        lerp(self.from, self.to, i, n)
    }
    fn settle(&mut self) {
        self.from = self.to;
    }
}

/// An effect plus what the host adds to every effect: on/off, input and output gain, dry/wet,
/// meters, and a delay on the dry signal so mixing it back lines up with the effect's latency.
struct Slot {
    effect: Box<dyn Effect>,
    enabled: Ramp,
    input: Ramp,
    output: Ramp,
    mix: Ramp,
    dry: Vec<f32>,
    delay: Vec<f32>,
    delay_pos: usize,
    input_peak: f32,
    output_peak: f32,
}

impl Slot {
    /// Off the audio thread: creates the effect and allocates its buffers.
    fn build(spec: &SlotSpec) -> Slot {
        let mut effect = (spec.info.create)();
        for (i, value) in spec.values[COMMON.len()..].iter().enumerate() {
            effect.set_param(i, *value);
        }
        effect.reset();
        debug_assert_eq!(effect.latency(), latency_of(spec), "{}: EffectInfo::latency disagrees with the effect", spec.info.kind);
        let v = &spec.values;
        Slot {
            delay: vec![0.0; effect.latency()],
            effect,
            enabled: Ramp::new(v[ENABLED]),
            input: Ramp::new(db_to_gain(v[INPUT_GAIN])),
            output: Ramp::new(db_to_gain(v[OUTPUT_GAIN])),
            mix: Ramp::new(v[MIX] / 100.0),
            dry: vec![0.0; FRAME],
            delay_pos: 0,
            input_peak: 0.0,
            output_peak: 0.0,
        }
    }

    fn set(&mut self, param: usize, value: f32) {
        match param {
            ENABLED => self.enabled.to = value,
            INPUT_GAIN => self.input.to = db_to_gain(value),
            OUTPUT_GAIN => self.output.to = db_to_gain(value),
            MIX => self.mix.to = value / 100.0,
            _ => self.effect.set_param(param - COMMON.len(), value),
        }
    }

    fn process(&mut self, frame: &mut [f32]) {
        let n = frame.len();
        for (i, s) in frame.iter_mut().enumerate() {
            *s *= self.input.at(i, n);
        }
        self.input_peak = peak(frame);

        if self.enabled.from == 0.0 && self.enabled.to == 0.0 {
            // Off: only the dry signal, delayed like the effect so the chain's latency stays put.
            delay(&mut self.delay, &mut self.delay_pos, frame);
        } else {
            if self.enabled.from == 0.0 {
                self.effect.reset(); // back on: no stale envelope from before it was turned off
            }
            let dry = &mut self.dry[..n];
            dry.copy_from_slice(frame);
            delay(&mut self.delay, &mut self.delay_pos, dry);
            self.effect.process(frame);
            for (i, (s, d)) in frame.iter_mut().zip(dry.iter()).enumerate() {
                let wet = self.enabled.at(i, n) * self.mix.at(i, n);
                *s = d + (*s - d) * wet;
            }
        }

        for (i, s) in frame.iter_mut().enumerate() {
            *s *= self.output.at(i, n);
        }
        self.output_peak = peak(frame);
        for ramp in [&mut self.enabled, &mut self.input, &mut self.output, &mut self.mix] {
            ramp.settle();
        }
    }
}

/// The latency the effect will have with these values (without creating it).
fn latency_of(spec: &SlotSpec) -> usize {
    (spec.info.latency)(&spec.values[COMMON.len()..])
}

/// Delays `samples` in place through the ring `line` (nothing when it's empty).
fn delay(line: &mut [f32], pos: &mut usize, samples: &mut [f32]) {
    if line.is_empty() {
        return;
    }
    for s in samples {
        std::mem::swap(s, &mut line[*pos]);
        *pos = (*pos + 1) % line.len();
    }
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0, |m, s| m.max(s.abs()))
}

/// The effects, built and ready to run.
pub struct Chain {
    slots: Vec<Slot>,
}

impl Chain {
    /// Off the audio thread.
    pub fn build(spec: &ChainSpec) -> Chain {
        Chain { slots: spec.slots.iter().map(Slot::build).collect() }
    }

    /// Samples of delay the whole chain adds.
    pub fn latency(&self) -> usize {
        self.slots.iter().map(|s| s.delay.len()).sum()
    }

    fn process(&mut self, frame: &mut [f32]) {
        for slot in &mut self.slots {
            slot.process(frame);
        }
    }
}

enum Command {
    Set { slot: usize, param: usize, value: f32 },
    Swap(Chain),
}

/// The audio-thread half. `process` once per frame.
pub struct ChainRunner {
    current: Chain,
    commands: Receiver<Command>,
    garbage: SyncSender<Chain>,
    retired: Vec<Chain>,
    scratch: Vec<f32>,
}

impl ChainRunner {
    /// Runs a frame (`FRAME` samples, mono) through the chain, in place.
    pub fn process(&mut self, frame: &mut [f32]) {
        debug_assert!(frame.len() <= FRAME);
        // Old chains go back to be freed elsewhere; while that queue is full they wait here.
        while let Some(old) = self.retired.pop() {
            match self.garbage.try_send(old) {
                Ok(()) => {}
                Err(TrySendError::Full(old)) => {
                    self.retired.push(old);
                    break;
                }
                Err(TrySendError::Disconnected(old)) => drop(old), // the app is closing
            }
        }

        let mut old = None;
        if self.retired.len() < RETIRED {
            while let Ok(command) = self.commands.try_recv() {
                match command {
                    Command::Set { slot, param, value } => {
                        if let Some(slot) = self.current.slots.get_mut(slot) {
                            slot.set(param, value);
                        }
                    }
                    Command::Swap(chain) => {
                        old = Some(std::mem::replace(&mut self.current, chain));
                        break; // one swap per frame; later messages are for the new chain
                    }
                }
            }
        }

        match old {
            None => self.current.process(frame),
            Some(mut old) => {
                // Both chains hear this frame; the output fades from the old one to the new one.
                let n = frame.len();
                let before = &mut self.scratch[..n];
                before.copy_from_slice(frame);
                old.process(before);
                self.current.process(frame);
                for (i, (s, b)) in frame.iter_mut().zip(before.iter()).enumerate() {
                    *s = b + (*s - b) * (i + 1) as f32 / n as f32;
                }
                self.retired.push(old);
            }
        }
    }

    pub fn latency(&self) -> usize {
        self.current.latency()
    }

    /// Input and output peaks (linear) of each effect in the last frame, in chain order.
    pub fn meters(&self) -> impl Iterator<Item = (f32, f32)> + '_ {
        self.current.slots.iter().map(|s| (s.input_peak, s.output_peak))
    }
}

/// The half off the audio thread: what the chain should be, and the way to tell the runner.
pub struct ChainControl {
    spec: ChainSpec,
    commands: Option<SyncSender<Command>>,
    garbage: Option<Receiver<Chain>>,
}

impl ChainControl {
    pub fn new(spec: ChainSpec) -> Result<ChainControl, String> {
        spec.validate()?;
        Ok(ChainControl { spec, commands: None, garbage: None })
    }

    pub fn spec(&self) -> &ChainSpec {
        &self.spec
    }

    /// A runner for the audio thread, built from the current spec. A previous runner stops getting
    /// changes (it keeps what it had).
    pub fn runner(&mut self) -> ChainRunner {
        let (commands_tx, commands) = sync_channel(QUEUE);
        let (garbage, garbage_rx) = sync_channel(RETIRED);
        self.commands = Some(commands_tx);
        self.garbage = Some(garbage_rx);
        ChainRunner {
            current: Chain::build(&self.spec),
            commands,
            garbage,
            retired: Vec::with_capacity(RETIRED),
            scratch: vec![0.0; FRAME],
        }
    }

    /// Sets parameter `key` ("enabled", "inputGain", "outputGain", "mix" or the effect's own) of
    /// the effect `slot_id`, clamped to its range. Returns the value actually set.
    pub fn set_param(&mut self, slot_id: &str, key: &str, value: f32) -> Result<f32, String> {
        let slot = self.spec.slots.iter().position(|s| s.id == slot_id).ok_or_else(|| format!("no effect {slot_id}"))?;
        let spec = &mut self.spec.slots[slot];
        let param = spec.param_index(key).ok_or_else(|| format!("{slot_id} has no parameter {key}"))?;
        let value = spec.specs().nth(param).expect("index from param_index").clamp(value);
        let latency = latency_of(spec);
        spec.values[param] = value;
        if latency_of(spec) != latency {
            // E.g. the limiter's lookahead: the chain's delays are sized when it's built.
            self.set_chain(self.spec.clone())?;
        } else {
            self.send(Command::Set { slot, param, value })?;
        }
        Ok(value)
    }

    /// Replaces the whole chain (reorder, add, remove, a preset): built here, crossfaded in by the
    /// runner on its next frame.
    pub fn set_chain(&mut self, spec: ChainSpec) -> Result<(), String> {
        spec.validate()?;
        let chain = Chain::build(&spec);
        self.spec = spec;
        self.send(Command::Swap(chain))
    }

    fn send(&mut self, command: Command) -> Result<(), String> {
        // Free the chains the runner is done with, here and not on the audio thread.
        if let Some(garbage) = &self.garbage {
            garbage.try_iter().for_each(drop);
        }
        let Some(commands) = &self.commands else { return Ok(()) };
        match commands.try_send(command) {
            Ok(()) => Ok(()),
            Err(TrySendError::Disconnected(_)) => {
                // No runner (the mic is closed): the spec already has the change for the next one.
                self.commands = None;
                self.garbage = None;
                Ok(())
            }
            Err(TrySendError::Full(_)) => Err("the audio thread isn't keeping up; try again".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    fn chain(kinds: &[&str]) -> ChainSpec {
        ChainSpec { slots: kinds.iter().enumerate().map(|(i, k)| SlotSpec::new(format!("{k}#{i}"), k).unwrap()).collect() }
    }

    fn sine(frame: usize) -> Vec<f32> {
        (frame * FRAME..(frame + 1) * FRAME).map(|i| 0.5 * (2.0 * PI * 440.0 * i as f32 / 48_000.0).sin()).collect()
    }

    /// The largest jump between neighbouring samples: a click shows up as a big one.
    fn max_step(out: &[f32]) -> f32 {
        out.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max)
    }

    #[test]
    fn reordering_100_times_during_a_sine_does_not_click() {
        // Gain then clip ≠ clip then gain, so a hard switch between the two orders jumps by up to
        // 0.3 in one sample; a sine through either order never steps more than ~0.058.
        let mut a = chain(&["test-gain", "test-clip"]);
        a.slots[0].values[COMMON.len()] = 6.0;
        let b = ChainSpec { slots: vec![a.slots[1].clone(), a.slots[0].clone()] };
        let mut control = ChainControl::new(a.clone()).unwrap();
        let mut runner = control.runner();
        let mut out = Vec::new();
        let mut swaps = 0;
        for f in 0..400 {
            if f % 4 == 0 && swaps < 100 {
                control.set_chain(if swaps % 2 == 0 { b.clone() } else { a.clone() }).unwrap();
                swaps += 1;
            }
            let mut frame = sine(f);
            runner.process(&mut frame);
            out.extend(frame);
        }
        assert_eq!(swaps, 100);
        let step = max_step(&out);
        assert!(step < 0.07, "a click: neighbouring samples {step} apart");
    }

    #[test]
    fn parameter_changes_ramp() {
        // Input gain flipping between -12 and +12 dB ten times a frame (1 000 times a second).
        let mut control = ChainControl::new(chain(&["test-gain"])).unwrap();
        let mut runner = control.runner();
        let mut out = Vec::new();
        for f in 0..100 {
            for k in 0..10 {
                control.set_param("test-gain#0", "inputGain", if (f + k) % 2 == 0 { -12.0 } else { 12.0 }).unwrap();
            }
            let mut frame = sine(f);
            runner.process(&mut frame);
            out.extend(frame);
        }
        // At +12 dB the sine itself steps up to 0.5 * 3.98 * 0.0576 ≈ 0.115.
        let step = max_step(&out);
        assert!(step < 0.13, "a click: neighbouring samples {step} apart");
    }

    #[test]
    fn turning_an_effect_on_and_off_does_not_click() {
        let mut control = ChainControl::new(chain(&["test-clip"])).unwrap();
        let mut runner = control.runner();
        let mut out = Vec::new();
        for f in 0..100 {
            control.set_param("test-clip#0", "enabled", (f % 2) as f32).unwrap();
            let mut frame = sine(f);
            runner.process(&mut frame);
            out.extend(frame);
        }
        let step = max_step(&out);
        assert!(step < 0.035, "a click: neighbouring samples {step} apart"); // the sine alone: 0.029
    }

    #[test]
    fn latency_adds_up_and_the_dry_signal_lines_up() {
        let mut spec = chain(&["test-delay", "test-delay"]);
        spec.slots[1].values[COMMON.len()] = 96.0;
        spec.slots[1].values[MIX] = 50.0; // half dry: misaligned, the impulse would show up twice
        let mut control = ChainControl::new(spec).unwrap();
        let mut runner = control.runner();
        assert_eq!(runner.latency(), 48 + 96);
        let mut out = Vec::new();
        for f in 0..2 {
            let mut frame = vec![0.0; FRAME];
            if f == 0 {
                frame[0] = 1.0;
            }
            runner.process(&mut frame);
            out.extend(frame);
        }
        let hits: Vec<(usize, f32)> = out.iter().copied().enumerate().filter(|(_, s)| s.abs() > 1e-6).collect();
        assert_eq!(hits.len(), 1, "one impulse out: {hits:?}");
        assert_eq!(hits[0].0, 144);
        assert!((hits[0].1 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn editing_a_chain_keeps_ids_unique() {
        let mut spec = ChainSpec::default();
        assert_eq!(spec.add("test-gain").unwrap(), "test-gain#0");
        assert_eq!(spec.add("test-clip").unwrap(), "test-clip#0");
        assert_eq!(spec.add("test-gain").unwrap(), "test-gain#1");
        spec.slots[0].values[INPUT_GAIN] = -3.0;
        assert_eq!(spec.duplicate("test-gain#0").unwrap(), "test-gain#2");
        let ids = |s: &ChainSpec| s.slots.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&spec), ["test-gain#0", "test-gain#2", "test-clip#0", "test-gain#1"]);
        assert_eq!(spec.slots[1].values[INPUT_GAIN], -3.0, "a copy keeps the values");
        spec.remove("test-gain#0").unwrap();
        assert_eq!(spec.add("test-gain").unwrap(), "test-gain#0", "a freed id is reused");
        spec.move_to("test-gain#0", 0).unwrap();
        spec.move_to("test-clip#0", 99).unwrap();
        assert_eq!(ids(&spec), ["test-gain#0", "test-gain#2", "test-gain#1", "test-clip#0"]);
        assert!(spec.validate().is_ok());
        assert!(spec.add("nope").is_err());
        assert!(spec.remove("nope").is_err());
        while spec.slots.len() < MAX_SLOTS {
            spec.add("test-clip").unwrap();
        }
        assert!(spec.add("test-clip").is_err() && spec.duplicate("test-clip#0").is_err(), "full");
        assert_eq!(chain(&["test-delay", "test-delay"]).latency(), 96);
    }

    #[test]
    fn a_parameter_that_changes_latency_rebuilds_the_chain() {
        let mut control = ChainControl::new(chain(&["limiter"])).unwrap();
        let mut runner = control.runner();
        assert_eq!(runner.latency(), 240);
        control.set_param("limiter#0", "lookahead", 10.0).unwrap();
        runner.process(&mut sine(0));
        assert_eq!(runner.latency(), 480, "the new chain is in");
        control.set_param("limiter#0", "ceiling", -3.0).unwrap(); // no latency change: just a message
        runner.process(&mut sine(1));
        assert_eq!(runner.latency(), 480);
    }

    #[test]
    fn bad_chains_are_refused() {
        let twice = ChainSpec { slots: vec![SlotSpec::new("x", "test-gain").unwrap(), SlotSpec::new("x", "test-clip").unwrap()] };
        assert!(ChainControl::new(twice).is_err(), "duplicate ids");
        let long = ChainSpec { slots: (0..=MAX_SLOTS).map(|i| SlotSpec::new(format!("g{i}"), "test-gain").unwrap()).collect() };
        assert!(ChainControl::new(long).is_err(), "too many effects");
        let mut control = ChainControl::new(chain(&["test-gain"])).unwrap();
        assert!(control.set_param("nope", "mix", 1.0).is_err());
        assert!(control.set_param("test-gain#0", "nope", 1.0).is_err());
        assert_eq!(control.set_param("test-gain#0", "mix", 500.0), Ok(100.0), "clamped");
        assert!(SlotSpec::new("a", "no-such-effect").is_none());
    }

    #[test]
    fn changes_without_a_runner_wait_in_the_spec() {
        let mut control = ChainControl::new(chain(&["test-gain"])).unwrap();
        drop(control.runner()); // the mic closed
        control.set_param("test-gain#0", "outputGain", -6.0).unwrap();
        control.set_chain(chain(&["test-gain", "test-clip"])).unwrap();
        assert_eq!(control.spec().slots.len(), 2);
        let runner = control.runner();
        assert_eq!(runner.meters().count(), 2, "the next runner starts from the spec");
    }

    #[test]
    fn the_audio_thread_does_not_allocate() {
        use crate::dsp::tests::allocations;
        let first = chain(&["test-gain", "test-clip", "test-delay"]);
        let s = &first.slots;
        let reordered = ChainSpec { slots: vec![s[1].clone(), s[0].clone(), s[2].clone()] };
        let mut control = ChainControl::new(first.clone()).unwrap();
        let mut runner = control.runner();
        let mut during = 0;
        for f in 0..200 {
            control.set_param("test-gain#0", "gain", (f % 13) as f32).unwrap();
            control.set_param("test-clip#1", "enabled", (f % 2) as f32).unwrap();
            if f % 10 == 0 {
                control.set_chain(if f % 20 == 0 { reordered.clone() } else { first.clone() }).unwrap();
            }
            let mut frame = sine(f);
            let before = allocations();
            runner.process(&mut frame);
            during += allocations() - before;
        }
        assert_eq!(during, 0, "allocations or frees on the audio thread");
    }
}
