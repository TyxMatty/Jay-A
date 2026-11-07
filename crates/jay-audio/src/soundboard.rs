//! The soundboard's mixer (docs/ARQUITETURA.md, section 7). Each destination (the call, mixed into
//! the mic after the effect chain; the headphones, an output of its own) has a `Mixer` on its own
//! audio thread, fed by a `Player` from any other thread. Clips are decoded once to 48 kHz stereo
//! (files::decode) and shared by `Arc`, so playing one copies nothing.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

/// A decoded clip: 48 kHz stereo f32, interleaved.
pub type Clip = Arc<[f32]>;

/// More at once and the oldest fades out.
pub const MAX_VOICES: usize = 8;
/// 10 ms: how a voice ends when it's cut (by a ninth voice or "stop all"), instead of a click.
const FADE: usize = 480;
/// Voices kept at most: the playing ones plus as many fading out.
const SLOTS: usize = MAX_VOICES * 2;

enum Command {
    Play(Clip, f32),
    StopAll,
}

/// Starts clips on a mixer, from any thread; never waits.
#[derive(Clone)]
pub struct Player(SyncSender<Command>);

impl Player {
    /// `gain`: linear (the clip's volume × the soundboard's).
    pub fn play(&self, clip: &Clip, gain: f32) {
        let _ = self.0.try_send(Command::Play(clip.clone(), gain));
    }

    pub fn stop_all(&self) {
        let _ = self.0.try_send(Command::StopAll);
    }
}

struct Voice {
    clip: Clip,
    /// Next frame.
    at: usize,
    gain: f32,
    /// Frames left of the fade-out, once cut.
    fading: Option<usize>,
}

/// The voices of one destination, mixed on its audio thread: no allocation and no lock there.
pub struct Mixer {
    commands: Receiver<Command>,
    voices: Vec<Voice>,
}

/// A mixer and the player that drives it.
pub fn mixer() -> (Player, Mixer) {
    let (tx, rx) = sync_channel(SLOTS);
    (Player(tx), Mixer { commands: rx, voices: Vec::with_capacity(SLOTS) })
}

impl Mixer {
    /// Adds the voices to `out`: `channels` 1 (the call: left and right averaged) or 2 (interleaved).
    // shortcut: a voice whose clip was removed from the library meanwhile frees the clip's memory
    // here when it ends; rare (deleting a clip while it plays), revisit if it ever shows up as a click.
    pub fn mix(&mut self, out: &mut [f32], channels: usize) {
        while let Ok(command) = self.commands.try_recv() {
            match command {
                Command::Play(clip, gain) => self.start(clip, gain),
                Command::StopAll => self.voices.iter_mut().for_each(cut),
            }
        }
        for voice in &mut self.voices {
            for frame in out.chunks_exact_mut(channels) {
                let Some(pair) = voice.clip.get(voice.at * 2..voice.at * 2 + 2) else { break };
                let fade = match &mut voice.fading {
                    Some(0) => break,
                    Some(left) => {
                        *left -= 1;
                        *left as f32 / FADE as f32
                    }
                    None => 1.0,
                };
                let g = voice.gain * fade;
                if channels == 1 {
                    frame[0] += (pair[0] + pair[1]) * 0.5 * g;
                } else {
                    frame[0] += pair[0] * g;
                    frame[1] += pair[1] * g;
                }
                voice.at += 1;
            }
        }
        self.voices.retain(|v| v.at * 2 < v.clip.len() && v.fading != Some(0));
    }

    fn start(&mut self, clip: Clip, gain: f32) {
        if self.voices.iter().filter(|v| v.fading.is_none()).count() >= MAX_VOICES {
            self.voices.iter_mut().filter(|v| v.fading.is_none()).take(1).for_each(cut); // the oldest
        }
        if self.voices.len() < SLOTS {
            self.voices.push(Voice { clip, at: 0, gain, fading: None });
        }
    }

    /// Voices playing or fading out.
    pub fn voices(&self) -> usize {
        self.voices.len()
    }
}

fn cut(voice: &mut Voice) {
    voice.fading.get_or_insert(FADE);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(frames: usize, value: f32) -> Clip {
        vec![value; frames * 2].into()
    }

    #[test]
    fn plays_a_clip_once_at_its_gain() {
        let (player, mut mixer) = mixer();
        player.play(&clip(1000, 0.8), 0.5);
        let mut out = vec![0.0; 2 * 1500];
        mixer.mix(&mut out, 2);
        assert!(out[..2000].iter().all(|&s| (s - 0.4).abs() < 1e-6));
        assert!(out[2000..].iter().all(|&s| s == 0.0), "and then stops");
        assert_eq!(mixer.voices(), 0);
    }

    #[test]
    fn the_call_gets_left_and_right_averaged() {
        let (player, mut mixer) = mixer();
        let stereo: Clip = [1.0f32, 0.0].repeat(10).into();
        player.play(&stereo, 1.0);
        let mut out = vec![0.0; 10];
        mixer.mix(&mut out, 1);
        assert!(out.iter().all(|&s| (s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn a_ninth_voice_fades_the_oldest_out_in_10_ms() {
        let (player, mut mixer) = mixer();
        let oldest = clip(48_000, 0.01);
        player.play(&oldest, 1.0);
        let mut out = vec![0.0; 2 * 480];
        mixer.mix(&mut out, 2);
        for _ in 0..MAX_VOICES {
            player.play(&clip(48_000, 0.0), 1.0);
        }
        let mut out = vec![0.0; 2 * 480];
        mixer.mix(&mut out, 2);
        assert!(out[0] > 0.0099 && out[out.len() - 1] < 0.0001, "faded: {} → {}", out[0], out[out.len() - 1]);
        assert!(out.windows(2).all(|w| w[1] <= w[0] + 1e-9), "smoothly");
        mixer.mix(&mut out, 2);
        assert_eq!(mixer.voices(), MAX_VOICES, "the oldest is gone, the eight new ones play");
    }

    #[test]
    fn stop_all_fades_everything_out() {
        let (player, mut mixer) = mixer();
        for _ in 0..3 {
            player.play(&clip(48_000, 0.1), 1.0);
        }
        let mut out = vec![0.0; 2 * 960];
        mixer.mix(&mut out, 2);
        player.stop_all();
        let mut out = vec![0.0; 2 * 960];
        mixer.mix(&mut out, 2);
        assert!(out[0] > 0.29 && out[2 * 480..].iter().all(|&s| s == 0.0), "silent after 10 ms");
        assert_eq!(mixer.voices(), 0);
    }

    #[test]
    fn mixing_never_allocates() {
        let (player, mut mixer) = mixer();
        let clips: Vec<Clip> = (0..20).map(|_| clip(4_800, 0.1)).collect();
        let mut out = vec![0.0; 2 * 480];
        let before = crate::dsp::tests::allocations();
        for (i, clip) in clips.iter().enumerate() {
            player.play(clip, 1.0); // more than SLOTS over time, with cuts and endings
            if i % 3 == 0 {
                player.stop_all();
            }
            for _ in 0..4 {
                mixer.mix(&mut out, 2);
            }
        }
        assert_eq!(crate::dsp::tests::allocations() - before, 0);
    }
}
