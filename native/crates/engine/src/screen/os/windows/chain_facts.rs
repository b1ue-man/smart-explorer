//! The capture facts of a Windows host process for the statistics (plan
//! 2026-11-14 „Bildkette“ BK0): path and fallback of `capture_source.rs`
//! (B-12) and whether Graphics Capture could limit its rate (B-11).
use super::capture_source::{fallback_bits, report, CapturePath, FallbackCause};
use crate::screen::core::frame_timing::{self, CaptureFacts, CaptureFallback};

/// What the session of this process captures with; `None` before a capture
/// path was chosen.
pub fn capture_facts() -> CaptureFacts {
    let report = report();
    let path = report.path.map(|path| match path {
        CapturePath::DesktopDuplication => frame_timing::CapturePath::DesktopDuplication,
        CapturePath::GraphicsCapture => frame_timing::CapturePath::WindowsGraphicsCapture,
    });
    let fallback = report.fallback.map(|cause| match cause {
        FallbackCause::Unsupported => CaptureFallback::DxgiUnsupported,
        FallbackCause::AccessLost => CaptureFallback::DxgiAccessLost,
        FallbackCause::AdapterChanged => CaptureFallback::DxgiAdapterChanged,
        FallbackCause::Failed => CaptureFallback::DxgiFailed,
    });
    CaptureFacts {
        path,
        fallback,
        rate_capped: path.map(|_| report.rate_capped),
        cursor_embedded: None,
        on_gpu: None,
    }
}

/// Plan 2026-11-17 P9: the failed step and result code of the fallback, for
/// the bits 24–63 of the packed facts (`FallbackDetail::pack`).
pub fn capture_fallback_bits() -> u64 {
    fallback_bits()
}
