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
use std::sync::{Mutex, PoisonError};

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

#[cfg(test)]
pub(crate) fn reset_for_test() {
    update(|report| *report = CaptureRouteReport::default());
}
