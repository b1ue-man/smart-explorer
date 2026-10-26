//! The Java heap offer is already narrowed by the app. It cannot prove a
//! larger native/GPU allocation class, so Android never supplies desktop facts.
pub(super) fn facts()->Option<crate::screen::core::frame_memory::MemoryFacts> {None}
