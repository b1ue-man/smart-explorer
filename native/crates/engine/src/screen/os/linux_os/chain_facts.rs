//! The capture facts of a Linux host process for the statistics (plan
//! 2026-11-14 „Bildkette“ BK0, r1): the path its capture noted in
//! `core::capture_route` (PipeWire DMA-BUF or shared memory from the
//! stream's format negotiation, X11 MIT-SHM or `GetImage`), why PipeWire
//! left DMA-BUF, and whether the pointer is part of the picture. The host
//! process packs them into its stream mark (`host_worker_encode.rs`); the
//! daemon takes them into `HostStats::chain` like the Windows facts.
use crate::screen::core::capture_route::{self, CaptureRoute, CaptureRouteReport};
use crate::screen::core::frame_timing::{CaptureFacts, CaptureFallback, CapturePath};

/// What the capture of this process reports of itself; `None` fields are
/// „not reported“ (before the first format, in a process without capture).
pub fn capture_facts() -> CaptureFacts {
    facts_of(&capture_route::report())
}

/// The facts of one `report` (apart for the tests).
pub(crate) fn facts_of(report: &CaptureRouteReport) -> CaptureFacts {
    let path = report.route.map(|route| match route {
        CaptureRoute::DmaBuf => CapturePath::PipewireDmabuf,
        CaptureRoute::SharedMemory => CapturePath::PipewireShm,
        CaptureRoute::X11Shm => CapturePath::Xshm,
        CaptureRoute::X11GetImage => CapturePath::X11GetImage,
    });
    CaptureFacts {
        path,
        fallback: report.fallback.and_then(CaptureFallback::from_key),
        // PipeWire and X11 deliver at the pace the host process asks for.
        rate_capped: None,
        cursor_embedded: report.cursor_embedded,
        on_gpu: None,
    }
}

/// Plan 2026-11-17 P9: the step and result code of a capture fallback for
/// the free bits of the packed facts (`capture_fallback::FallbackDetail`);
/// this platform reports none yet.
pub fn capture_fallback_bits() -> u64 {
    0
}

#[cfg(test)]
#[path = "bildkette_r1_capture_task_tests.rs"]
mod bildkette_r1_capture_task_tests;
