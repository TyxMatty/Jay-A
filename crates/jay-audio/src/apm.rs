//! Safe wrapper over WebRTC's AudioProcessing via the C shim in native/jayc_apm.cpp.

use std::ffi::c_void;

unsafe extern "C" {
    fn jayc_apm_create(
        sample_rate: i32,
        capture_channels: i32,
        render_channels: i32,
        echo_cancellation: i32,
        noise_suppression: i32,
        auto_gain: i32,
    ) -> *mut c_void;
    fn jayc_apm_process_render(apm: *mut c_void, interleaved: *const f32, frames: i32) -> i32;
    fn jayc_apm_process_capture(apm: *mut c_void, interleaved: *mut f32, frames: i32, delay_ms: i32) -> i32;
    fn jayc_apm_destroy(apm: *mut c_void);
}

/// Echo cancellation + noise suppression + automatic gain, on 10 ms frames.
pub struct Apm {
    handle: *mut c_void,
    capture_channels: usize,
    render_channels: usize,
    frames: usize,
}

// The APM is internally synchronized; we only ever use it from one thread at a time anyway.
unsafe impl Send for Apm {}

impl Apm {
    pub fn new(
        sample_rate: u32,
        capture_channels: usize,
        render_channels: usize,
        echo_cancellation: bool,
        noise_suppression: bool,
        auto_gain: bool,
    ) -> Option<Self> {
        let handle = unsafe {
            jayc_apm_create(
                sample_rate as i32,
                capture_channels as i32,
                render_channels as i32,
                echo_cancellation as i32,
                noise_suppression as i32,
                auto_gain as i32,
            )
        };
        (!handle.is_null()).then(|| Apm { handle, capture_channels, render_channels, frames: sample_rate as usize / 100 })
    }

    /// Samples (interleaved) in one 10 ms capture frame.
    pub fn capture_frame_len(&self) -> usize {
        self.frames * self.capture_channels
    }

    /// Samples (interleaved) in one 10 ms render (echo reference) frame.
    pub fn render_frame_len(&self) -> usize {
        self.frames * self.render_channels
    }

    /// Feeds 10 ms of what the speakers play.
    pub fn process_render(&mut self, frame: &[f32]) -> bool {
        assert_eq!(frame.len(), self.render_frame_len());
        unsafe { jayc_apm_process_render(self.handle, frame.as_ptr(), self.frames as i32) == 0 }
    }

    /// Cleans 10 ms of microphone audio in place.
    pub fn process_capture(&mut self, frame: &mut [f32], delay_ms: i32) -> bool {
        assert_eq!(frame.len(), self.capture_frame_len());
        unsafe { jayc_apm_process_capture(self.handle, frame.as_mut_ptr(), self.frames as i32, delay_ms) == 0 }
    }
}

impl Drop for Apm {
    fn drop(&mut self) {
        unsafe { jayc_apm_destroy(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::Apm;

    const RATE: usize = 48_000;
    const FRAME: usize = RATE / 100;

    /// Speech-like far-end signal: 250 ms bursts of broadband noise shaped by a slow envelope,
    /// alternating with silence. (Pure tones are too narrowband for the adaptive filter to learn
    /// the echo path, and real voices are broadband.)
    fn far_end(seconds: usize) -> Vec<f32> {
        let mut seed = 1u32;
        let mut noise = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        let mut lowpassed = 0f32;
        (0..seconds * RATE)
            .map(|i| {
                let burst = i % (RATE / 2);
                if burst < RATE / 4 {
                    let envelope = (std::f32::consts::PI * burst as f32 / (RATE / 4) as f32).sin();
                    lowpassed = 0.7 * lowpassed + 0.3 * noise(); // tilt energy towards voice frequencies
                    0.6 * envelope * lowpassed
                } else {
                    0.0
                }
            })
            .collect()
    }

    /// Plays `far` as the render (stereo) and an echo of it (40 ms later, -6 dB) as the mic;
    /// returns the mic output energy (dB) over the last 3 seconds.
    fn residual_echo_db(echo_cancellation: bool) -> f32 {
        let seconds = 10;
        let far = far_end(seconds);
        let delay = RATE * 40 / 1000;
        let mut apm = Apm::new(RATE as u32, 1, 2, echo_cancellation, false, false).expect("apm");
        let mut energy = 0f64;
        let mut count = 0usize;
        for start in (0..far.len() - FRAME).step_by(FRAME) {
            let render: Vec<f32> = far[start..start + FRAME].iter().flat_map(|&s| [s, s]).collect();
            assert!(apm.process_render(&render));
            let mut mic: Vec<f32> = (start..start + FRAME)
                .map(|i| if i >= delay { 0.5 * far[i - delay] } else { 0.0 })
                .collect();
            assert!(apm.process_capture(&mut mic, 0));
            if start >= far.len() - 3 * RATE {
                energy += mic.iter().map(|&s| (s as f64).powi(2)).sum::<f64>();
                count += FRAME;
            }
        }
        (10.0 * (energy / count as f64 + 1e-12).log10()) as f32
    }

    #[test]
    fn echo_canceller_removes_the_far_end_from_the_mic() {
        let without = residual_echo_db(false);
        let with = residual_echo_db(true);
        println!("residual echo: {without:.1} dB without AEC, {with:.1} dB with AEC ({:.1} dB less)", without - with);
        assert!(without > -40.0, "the echo should be clearly present without AEC ({without:.1} dB)");
        assert!(without - with > 20.0, "AEC should remove at least 20 dB of echo ({without:.1} -> {with:.1} dB)");
    }
}
