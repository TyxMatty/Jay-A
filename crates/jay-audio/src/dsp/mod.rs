//! The user's effect chain (docs/ARQUITETURA.md, section 5): what parameters accept, the effects,
//! the rack that runs them on the audio thread without ever waiting, and presets.

pub mod chain;
pub mod effect;
pub mod params;
pub mod preset;

/// 10 ms at 48 kHz: the chain's frame, the same as WebRTC's audio processing.
pub const FRAME: usize = crate::SAMPLE_RATE as usize / 100;

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
