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
//! disarmed then and the capture goes back to the CPU route.
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
