//! The colour conversion time of one encoding, apart from the encoding
//! itself (plan 2026-11-14 „Bildkette“ 4.10a, station `convert`).
//!
//! Whoever measures the station `encode` arms the clock of its thread
//! (`arm`) right before it hands a picture to the encoder and takes the sum
//! afterwards (`take`). A conversion that is its own call and ends
//! synchronously on that thread measures itself with `Timed`:
//!
//! - the CPU conversions BGRA → I420/NV12 of `pixel_convert` (OpenH264,
//!   SVT-AV1, the I420 input of the Linux hardware encoders, the CPU input of
//!   Media Foundation);
//! - the DMA-BUF import of the Linux VA encoders (`VaEncodeBase::
//!   import_dmabuf`: import, video processor and `vaSyncSurface`).
//!
//! Conversions inside the encoder or queued on the GPU without a wait of
//! their own stay in `encode` (`docs/oberflaechen/bildschirm.md`). A
//! conversion on a thread whose clock is not armed (a capture thread, the
//! viewer) records nothing, so nothing outside an encoding is counted.
use std::cell::Cell;
use std::time::Instant;

thread_local! {
    /// `None`: not armed; `Some(None)`: armed, nothing converted yet.
    static CLOCK: Cell<Option<Option<u64>>> = const { Cell::new(None) };
}

/// Starts the conversion sum of the next encoding on this thread.
pub fn arm() {
    CLOCK.with(|clock| clock.set(Some(None)));
}

/// The conversion time (µs) since `arm`, and disarms; `None` when no
/// separate conversion ran (it is then part of `encode`, or there is none).
pub fn take() -> Option<u64> {
    CLOCK.with(|clock| clock.replace(None)).flatten()
}

/// Adds `micros` to an armed clock of this thread.
pub fn add(micros: u64) {
    CLOCK.with(|clock| {
        if let Some(sum) = clock.get() {
            clock.set(Some(Some(sum.unwrap_or(0).saturating_add(micros))));
        }
    });
}

/// Measures one conversion until it is dropped.
pub struct Timed(Instant);

impl Timed {
    pub fn start() -> Self {
        Self(Instant::now())
    }
}

impl Drop for Timed {
    fn drop(&mut self) {
        add(u64::try_from(self.0.elapsed().as_micros()).unwrap_or(u64::MAX));
    }
}

/// The station `encode` without a separately measured conversion.
pub fn encode_without(total_us: u64, convert_us: Option<u64>) -> u64 {
    total_us.saturating_sub(convert_us.unwrap_or(0))
}

#[cfg(test)]
#[path = "bildkette_r1_chain_task_tests.rs"]
mod bildkette_r1_chain_task_tests;
