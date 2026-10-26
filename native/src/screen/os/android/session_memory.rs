//! The Java heap offer is already narrowed by the app. It cannot prove a
//! larger native/GPU allocation class, so Android never supplies desktop facts.
pub(super) fn facts()->Option<crate::screen::core::frame_memory::MemoryFacts> {None}
pub(super) fn preview_admission()->crate::screen::core::frame_budget::PreviewAdmission {
    crate::screen::core::frame_budget::PreviewAdmission::Geometric
}

use std::sync::atomic::{AtomicU64, Ordering};
use crate::screen::core::frame_memory::{CapacityCeiling, MemoryFacts};

static HEAP_OFFER: AtomicU64 = AtomicU64::new(0);

pub(super) fn ceiling(_facts: Option<MemoryFacts>) -> CapacityCeiling {
    let bytes = HEAP_OFFER.load(Ordering::Acquire);
    // Preserve the existing legacy fallback only when the app has not supplied
    // its heap policy. A negotiated small frame request is never that policy.
    CapacityCeiling::bounded(if bytes == 0 {
        crate::screen::core::frame_budget::MAX_WORKING_BYTES
    } else {bytes})
}

pub(super) fn observe_ceiling(ceiling: CapacityCeiling) {
    HEAP_OFFER.store(ceiling.bytes(), Ordering::Release);
}
