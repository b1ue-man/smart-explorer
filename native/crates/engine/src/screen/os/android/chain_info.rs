//! The capture facts of the Android host for the statistics (plan
//! 2026-11-14 „Bildkette“ BK0, r1): the app captures with MediaProjection
//! into the encoder's input surface; the core runs in the daemon process
//! and takes these facts in `BridgeSource::drain_chain` (`bridge_source.rs`).
//! Pointer and rate cap: the app reports neither, so they stay „not
//! reported“.
use crate::screen::core::frame_timing::{CaptureFacts, CapturePath};

/// MediaProjection, nothing else reported.
pub fn capture_facts() -> CaptureFacts {
    CaptureFacts {
        path: Some(CapturePath::MediaProjection),
        ..CaptureFacts::default()
    }
}
