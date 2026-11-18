//! What the capture of this process does with its pictures, for the
//! statistics (plan 2026-11-14 „Bildkette“ 4.10a; BK8b hand-off Ü4, BK8a
//! result 1, merge W2): the memory the stream negotiated (DMA-BUF or shared
//! memory), why it left DMA-BUF, and whether the portal draws the pointer
//! into the picture (`cursor_embedded`); for X11 whether MIT-SHM carries
//! the pictures. One value per process: a host process captures one source;
//! the daemon has none of its own on Linux. The platform notes the values
//! when they change (`os/linux_os`: `pipewire_dmabuf_stream.rs`,
//! `portal_source_binding.rs`, `x11_capture.rs`); `os/linux_os/
//! chain_facts.rs` reads `report()` into the `CaptureFacts` of the host
//! process's stream mark, from there into `HostStats::chain` and `se screen
//! sessions --json` (r1). Platform-neutral data only.
//!
//! Plan 2026-11-17 P9: `pointer_patch`, the area a host process redraws
//! when it blends the pointer into a picture that stays on the GPU (Windows
//! Desktop Duplication, `os/windows/capture_pointer_gpu.rs`).
use std::sync::{Mutex, PoisonError};

use super::pixel_convert::{FrameSize, Rect};

/// The memory the picture stream of the capture uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureRoute {
    /// The PipeWire buffers in shared memory, copied into CPU pixels.
    SharedMemory,
    /// The PipeWire buffers as DMA-BUF, imported by the GPU encoder.
    DmaBuf,
    /// X11 pictures through a MIT-SHM segment.
    X11Shm,
    /// X11 pictures with plain `GetImage` (the server has no usable MIT-SHM).
    X11GetImage,
}

impl CaptureRoute {
    pub fn key(self) -> &'static str {
        match self {
            Self::SharedMemory => "shm",
            Self::DmaBuf => "dmabuf",
            Self::X11Shm => "xshm",
            Self::X11GetImage => "x11_get_image",
        }
    }
}

/// The values a later step takes into the statistics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CaptureRouteReport {
    /// The negotiated memory of the running stream; `None` before the
    /// first format or on a capture without PipeWire (X11, Windows).
    pub route: Option<CaptureRoute>,
    /// Why the stream left DMA-BUF (`dmabuf_modifier`, `dmabuf_layout`,
    /// `dmabuf_import`, `dmabuf_conversion`, `dmabuf_consumer_cpu`,
    /// `dmabuf_pointer_blend`); `None` while it did not.
    pub fallback: Option<&'static str>,
    /// The pointer is part of the picture (the portal offered no pointer
    /// metadata, BK8a): the viewer cannot hide or draw it itself. `None`
    /// until the capture chose its pointer mode.
    pub cursor_embedded: Option<bool>,
}

static REPORT: Mutex<CaptureRouteReport> = Mutex::new(CaptureRouteReport {
    route: None,
    fallback: None,
    cursor_embedded: None,
});

fn update(change: impl FnOnce(&mut CaptureRouteReport)) {
    change(&mut REPORT.lock().unwrap_or_else(PoisonError::into_inner));
}

/// The values of this process now.
pub fn report() -> CaptureRouteReport {
    REPORT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// The stream negotiated `route`; a new DMA-BUF negotiation clears an old
/// fallback cause (a new session of the process).
pub fn note_route(route: CaptureRoute) {
    update(|report| {
        if route == CaptureRoute::DmaBuf {
            report.fallback = None;
        }
        report.route = Some(route);
    });
}

/// The stream left DMA-BUF because of `cause`; the first cause stays.
pub fn note_fallback(cause: &'static str) {
    update(|report| {
        report.fallback.get_or_insert(cause);
        report.route = Some(CaptureRoute::SharedMemory);
    });
}

/// The capture chose its pointer mode (`cursor_embedded` or not).
pub fn note_cursor_embedded(embedded: bool) {
    update(|report| report.cursor_embedded = Some(embedded));
}

/// The area of a picture of `native` size (its even part: the NV12 picture
/// on the GPU has even sides) that a pointer of `width` x `height` with its
/// top left at (`left`, `top`) covers, widened to even edges so every 2 x 2
/// chroma pair lies wholly inside or outside it; `None` when the pointer
/// lies outside that part or has no pixels.
pub fn pointer_patch(
    native: FrameSize,
    left: i32,
    top: i32,
    width: u32,
    height: u32,
) -> Option<Rect> {
    let span = |start: i32, length: u32, side: u32| -> Option<(u32, u32)> {
        let limit = i64::from(side & !1);
        let first = i64::from(start).max(0) & !1;
        let end = (i64::from(start) + i64::from(length)).min(limit);
        let end = (end + 1) & !1;
        (length > 0 && end > first).then_some((first as u32, (end - first) as u32))
    };
    let (x, width) = span(left, width, native.width)?;
    let (y, height) = span(top, height, native.height)?;
    Some(Rect {
        x,
        y,
        width,
        height,
    })
}

#[cfg(test)]
#[path = "capture_route_tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) fn reset_for_test() {
    update(|report| *report = CaptureRouteReport::default());
}
