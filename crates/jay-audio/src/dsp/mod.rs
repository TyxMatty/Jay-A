//! The user's effect chain (docs/ARQUITETURA.md, section 5): what parameters accept, the effects,
//! the rack that runs them on the audio thread without ever waiting, and presets.

pub mod chain;
pub mod effect;
pub mod effects;
pub mod params;
pub mod preset;

/// 10 ms at 48 kHz: the chain's frame, the same as WebRTC's audio processing.
pub const FRAME: usize = crate::SAMPLE_RATE as usize / 100;

/// The fixed limiter at the end of the microphone path (-1 dBFS, 5 ms lookahead): voice plus
/// soundboard never go over, whatever the user's chain does (docs/ARQUITETURA.md, section 5).
pub fn output_limiter() -> Box<dyn effect::Effect> {
    (effects::limiter::INFO.create)()
}

/// What the meters show, sent from the audio thread every 20 ms: plain numbers in a fixed-size
/// value, so sending it allocates nothing.
#[derive(Debug, Clone, Copy)]
pub struct Meters {
    /// RMS level of what went out, dBFS (-inf for silence).
    pub level: f32,
    /// Input and output peaks (linear) of the chain's effects, `count` of them, in order.
    pub effects: [(f32, f32); chain::MAX_SLOTS],
    pub count: usize,
}

/// `a` → `b` over a frame of `n` samples: the value for sample `i` (reaches `b` on the last one).
fn lerp(a: f32, b: f32, i: usize, n: usize) -> f32 {
    a + (b - a) * (i + 1) as f32 / n as f32
}

#[cfg(test)]
pub(crate) mod tests {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    /// Counts the allocations and frees each thread makes, so a test can check the audio thread's
    /// code makes none (RNF-3).
    struct Counting;

    thread_local! {
        static COUNT: Cell<usize> = const { Cell::new(0) };
    }

    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            COUNT.with(|c| c.set(c.get() + 1));
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            COUNT.with(|c| c.set(c.get() + 1));
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    #[global_allocator]
    static ALLOCATOR: Counting = Counting;

    /// Allocations and frees made so far by this thread.
    pub fn allocations() -> usize {
        COUNT.with(Cell::get)
    }
}
