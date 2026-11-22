//! The bridge after its armed encoder went (review 2026-11-19, Gemini
//! round 8, 1.2). The NV12 ring belongs to the capture (`Bridge::ring`,
//! made and retired by `capture_gpu_route` and `route_capture`), not to an
//! encoder; an encoder only arms the GPU route (`attach`). Before, the drop
//! of a `GpuInput` disarmed it at once: a capture in the gap until the next
//! encoder of the same process armed again took the CPU route and retired
//! the ring, and a key-frame request on the kept ring picture then found it
//! gone (`RingPicture::held`) and renewed the capture (`capture_thread`,
//! `renew`): a new generation and one more new encoder after every encoder
//! change. Now the route stays armed for `RELEASE_LINGER` after the drop; a
//! new encoder arms it again (`attach` clears `released`), else it is
//! disarmed then, or at once when a picture of it is read back (`not_taken`),
//! and the capture goes back to the CPU route.
//!
//! No GPU resource is freed while in use: the dropped encoder's input slot
//! is finished (or abandoned, `GpuInput::abandon`) before, its textures go
//! with it, and the ring itself is retired only by `route_capture` on a CPU
//! route, which waits (bounded) for every picture of it (`retire`).
use super::*;

/// How long the route stays armed for a following encoder.
const RELEASE_LINGER: Duration = Duration::from_secs(2);

/// The armed encoder's lingering ends after `RELEASE_LINGER` (called with
/// the bridge locked, before the route of a picture is decided).
pub(super) fn lingered(state: &mut Bridge) {
    if state
        .released
        .is_some_and(|at| at.elapsed() >= RELEASE_LINGER)
    {
        state.armed = None;
        state.released = None;
    }
}

/// Review 2026-11-19, round 9 H1: a ring picture went to a consumer that
/// reads it back (a CPU encoder, the system-memory MFT, an encoder on a
/// device of its own): the encoder that follows does not take the ring, so
/// a lingering route ends at once and the capture takes the CPU route from
/// the next picture (before, 2 s of NV12 4:2:0 pictures under „Verlustfrei“,
/// or pictures an encoder on its own device dropped as gone). An armed
/// route stays: only a GPU encoder that arms it reads the ring otherwise.
pub(in crate::screen::os) fn not_taken() {
    let mut state = bridge();
    if state.released.take().is_some() {
        state.armed = None;
    }
}

impl GpuInput {
    /// Codex round 6, finding 3: the ring slot in flight stays taken for good
    /// (an encoder whose driver may still read it); the capture uses the
    /// other. Round 9 H7: only that slot (never released) and its texture
    /// are kept, not the ring (`retire` waited 250 ms for its leaked `Arc`
    /// on the capture thread, its frame acquired, at every route change).
    /// Codex round 9, 2: a picture that timed out keeps its ring slot
    /// (`finish` leaves it, `texture` refuses) until the session went:
    /// `late_done` when its driver work finished, else `abandon`.
    pub(in crate::screen::os) fn keep_late(&mut self) {
        self.late = true;
    }

    pub(in crate::screen::os) fn late_done(&mut self) {
        self.late = false;
    }

    /// Codex round 9, 2: the ring is spoiled, so the capture replaces it
    /// with a ring of two free slots instead of writing into this one.
    pub(in crate::screen::os) fn abandon(&mut self) {
        if let Some((ring, _, texture)) = self.in_flight.take() {
            ring.spoil();
            std::mem::forget(texture);
        }
    }
}

impl Drop for GpuInput {
    fn drop(&mut self) {
        self.finish();
        let Some(own) = self.token else {
            return;
        };
        let mut state = bridge();
        if state.armed.map(|(_, token)| token) == Some(own) {
            state.released = Some(Instant::now());
        }
    }
}
