//! Hooks of the host process into the platform modules (moved out of
//! `mod.rs`).

/// The source a host process opened, where its input points (SCR-M6); the
/// Wayland portal session keeps its own.
#[cfg(windows)]
pub(super) fn note_input_target(source: &crate::screen::core::window_list::SourceId) {
    super::input_send::set_target(source);
}

#[cfg(target_os = "linux")]
pub(super) fn note_input_target(source: &crate::screen::core::window_list::SourceId) {
    super::x11_input::set_target(source);
}

#[cfg(not(any(windows, target_os = "linux")))]
pub(super) fn note_input_target(_source: &crate::screen::core::window_list::SourceId) {}

/// Enables PipeWire DMA-BUF offers in the host worker process (BK8b, W2).
#[cfg(target_os = "linux")]
pub(super) fn enable_gpu_pictures() {
    super::pipewire_stream::dmabuf::enable_gpu_pipeline(true);
}

#[cfg(not(target_os = "linux"))]
pub(super) fn enable_gpu_pictures() {}
