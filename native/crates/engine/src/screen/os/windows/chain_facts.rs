//! The capture facts of a Windows host process for the statistics (plan
//! 2026-11-14 „Bildkette“ BK0): path and fallback of `capture_source.rs`
//! (B-12) and whether Graphics Capture could limit its rate (B-11). Plan
//! 2026-11-17 P9: without a fallback of the capture path, the fallback names
//! why the last picture went to the encoder as CPU pixels
//! (`capture_gpu_route.rs` `CpuReason`); the report shows it only beside
//! `convert cpu` (`chain_meter::shown_fallback`).
use super::capture_readback::{last_cpu_reason, CpuReason};
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
    let fallback = report
        .fallback
        .map(|cause| match cause {
            FallbackCause::Unsupported => CaptureFallback::DxgiUnsupported,
            FallbackCause::AccessLost => CaptureFallback::DxgiAccessLost,
            FallbackCause::AdapterChanged => CaptureFallback::DxgiAdapterChanged,
            FallbackCause::Failed => CaptureFallback::DxgiFailed,
        })
        .or_else(|| last_cpu_reason().and_then(conversion_cause));
    CaptureFacts {
        path,
        fallback,
        rate_capped: path.map(|_| report.rate_capped),
        cursor_embedded: None,
        on_gpu: None,
    }
}

/// The chain's name of a reason for CPU pixels; a still image's exact
/// readback is none.
pub(super) fn conversion_cause(reason: CpuReason) -> Option<CaptureFallback> {
    Some(match reason {
        CpuReason::NotArmed => CaptureFallback::GpuNotArmed,
        CpuReason::OtherDevice => CaptureFallback::GpuOtherDevice,
        CpuReason::Disabled => CaptureFallback::GpuDisabled,
        CpuReason::NoVideo => CaptureFallback::GpuNoVideo,
        CpuReason::Rotated => CaptureFallback::GpuRotated,
        CpuReason::Scaled => CaptureFallback::GpuScaled,
        CpuReason::PointerBlended => CaptureFallback::GpuPointerBlend,
        CpuReason::Exact => return None,
    })
}

/// Plan 2026-11-17 P9: the failed step and result code of the fallback, for
/// the bits 24–63 of the packed facts (`FallbackDetail::pack`).
pub fn capture_fallback_bits() -> u64 {
    fallback_bits()
}
