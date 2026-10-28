//! Platform side of the screen strand (S4). SCR-M2 brought the parts every
//! platform shares: the test-picture backend and the indicator window of the
//! desktop worker. SCR-M3 adds the codec inventory of a process, the host
//! process that captures and encodes per session (its client in the daemon,
//! its main loop, its lifetime per OS: a job object on Windows, the
//! parent-death signal on Linux), Media Foundation on Windows (loaded at run
//! time) and OpenH264 on Windows and Linux (`native_ffi`, not on Android).
//! SCR-M4 adds the Windows host (`windows/`): monitors, the window list, the
//! secure desktop, and capture with Graphics Capture or the DXGI duplication.
//! SCR-M5 adds the Linux host (`linux_os/`): X11, the Wayland portal with
//! PipeWire, window and app lists per desktop. SCR-M8 adds the Android host
//! (`android/`): the bridge to the app, which captures, encodes and injects.
//! The functions below select the platform's parts with `#[cfg(...)]` so the
//! shared modules stay free of it.
use std::path::Path;
use std::sync::Arc;

use crate::screen::core::codec_registry::{DecoderFactory, EncoderFactory, OpenH264Report};
use crate::screen::core::host_backend::{ScreenHostBackend, SessionIndicator};
use crate::screen::core::registry::ScreenRegistry;

#[path = "shared/image_memory.rs"]
pub(crate) mod image_memory;
#[cfg(target_os = "linux")]
#[path = "linux_os/session_memory.rs"]
mod platform_session_memory;
#[cfg(windows)]
#[path = "windows/session_memory.rs"]
mod platform_session_memory;
#[cfg(not(any(windows, target_os = "linux")))]
#[path = "android/session_memory.rs"]
mod platform_session_memory;
#[path = "shared/session_memory.rs"]
pub(crate) mod session_memory;

#[cfg(any(target_os = "android", test))]
#[path = "android/app_icons.rs"]
pub(crate) mod android_app_icons;
#[path = "shared/app_icons.rs"]
pub(crate) mod app_icon_raster;
#[cfg(windows)]
#[path = "windows/app_icons.rs"]
pub(crate) mod app_icons;
#[cfg(target_os = "linux")]
#[path = "linux_os/app_icons.rs"]
pub(crate) mod app_icons;
#[cfg(target_os = "android")]
pub(crate) use android_app_icons as app_icons;

#[cfg(windows)]
#[path = "windows/login_source.rs"]
mod login_source;
#[cfg(target_os = "linux")]
#[path = "linux_os/login_source.rs"]
mod login_source;
#[cfg(any(windows, target_os = "linux"))]
pub(crate) use login_source::{
    inject_login_input, login_geometry_current, open_login_source, prepare_login_source,
};
#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) fn login_geometry_current() -> std::io::Result<bool> {
    Ok(false)
}
#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) fn open_login_source(
) -> std::io::Result<Box<dyn crate::screen::core::host_backend::ScreenSource>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "login capture is unavailable on this platform",
    ))
}
#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) fn prepare_login_source(
    _admission: Arc<crate::screen::core::capture_admission::SessionAdmission>,
) -> std::io::Result<Box<dyn crate::screen::core::capture_source_contract::PreparedSource>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "login capture is unavailable on this platform",
    ))
}
#[cfg(not(any(windows, target_os = "linux")))]
pub(crate) fn inject_login_input(
    _events: &[crate::screen::core::input_model::InputEvent],
) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "login input is unavailable on this platform",
    ))
}

#[path = "shared/codec_inventory.rs"]
pub(crate) mod codec_inventory;
// Android has no host process (the app captures): the client stays unused there.
#[cfg_attr(target_os = "android", allow(dead_code))]
#[path = "shared/host_worker_client.rs"]
pub(crate) mod host_worker_client;
#[path = "shared/host_worker_main.rs"]
pub(crate) mod host_worker_main;
// Android shows sessions in the app (`bridge_backend::BridgeIndicator`); the
// desktop window is built only in share-node tests there.
#[cfg_attr(target_os = "android", allow(dead_code))]
#[path = "shared/indicator.rs"]
pub(crate) mod indicator;
#[path = "shared/synthetic_backend.rs"]
pub(crate) mod synthetic_backend;
// SCR-M6: key codes of the desktop hosts' input (plain tables).
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
#[path = "shared/input_keys.rs"]
pub(crate) mod input_keys;

#[cfg(target_os = "linux")]
#[path = "linux_os/worker_death.rs"]
mod worker_lifetime;
#[cfg(windows)]
#[path = "windows/worker_job.rs"]
mod worker_lifetime;
/// Elsewhere the host process ends with the end of its input alone.
#[cfg(not(any(windows, target_os = "linux")))]
mod worker_lifetime {
    pub(super) fn prepare(_command: &mut std::process::Command) {}

    pub(super) fn bind(_child: &std::process::Child) -> std::io::Result<()> {
        Ok(())
    }

    pub(super) fn arm_worker() -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(all(test, any(windows, target_os = "linux")))]
#[path = "native_ffi/batches_capture_codec_task_tests.rs"]
mod batches_capture_openh264_task_tests;
#[cfg(any(windows, target_os = "linux"))]
#[path = "native_ffi/openh264_capture.rs"]
mod openh264_capture;
#[cfg(any(windows, target_os = "linux"))]
#[path = "native_ffi/openh264_codec.rs"]
pub(crate) mod openh264_codec;
#[cfg(any(windows, target_os = "linux"))]
#[path = "native_ffi/openh264_encode.rs"]
mod openh264_encode;
#[cfg(any(windows, target_os = "linux"))]
#[path = "native_ffi/openh264_install.rs"]
pub(crate) mod openh264_install;
#[cfg(any(windows, target_os = "linux"))]
#[path = "native_ffi/openh264_pins.rs"]
pub(crate) mod openh264_pins;

#[cfg(target_os = "linux")]
#[path = "linux_os/app_units.rs"]
mod app_units;
#[cfg(all(test, windows))]
#[path = "windows/batches_capture_codec_task_tests.rs"]
mod batches_capture_mf_task_tests;
#[cfg(windows)]
#[path = "windows/capture_dxgi.rs"]
mod capture_dxgi;
#[cfg(windows)]
#[path = "windows/capture_prepared.rs"]
mod capture_prepared;
#[cfg(windows)]
#[path = "windows/capture_readback.rs"]
mod capture_readback;
#[cfg(windows)]
#[path = "windows/capture_retirement.rs"]
mod capture_retirement;
#[cfg(windows)]
#[path = "windows/capture_slots.rs"]
mod capture_slots;
#[cfg(windows)]
#[path = "windows/capture_source.rs"]
mod capture_source;
#[cfg(windows)]
#[path = "windows/capture_wgc.rs"]
mod capture_wgc;
#[cfg(windows)]
#[path = "windows/d3d_device.rs"]
mod d3d_device;
#[cfg(target_os = "linux")]
#[path = "linux_os/host_backend.rs"]
mod host_backend;
#[cfg(windows)]
#[path = "windows/host_backend.rs"]
mod host_backend;
#[cfg(windows)]
#[path = "windows/input_integrity.rs"]
mod input_integrity;
#[cfg(windows)]
#[path = "windows/input_send.rs"]
mod input_send;
#[cfg(target_os = "linux")]
#[path = "linux_os/kde_window_management.rs"]
mod kde_window_management;
#[cfg(windows)]
#[path = "windows/mf_buffer_lease.rs"]
mod mf_buffer_lease;
#[cfg(windows)]
#[path = "windows/mf_capture_media.rs"]
mod mf_capture_media;
#[cfg(windows)]
#[path = "windows/mf_capture_output.rs"]
mod mf_capture_output;
#[cfg(windows)]
#[path = "windows/mf_common.rs"]
mod mf_common;
#[cfg(windows)]
#[path = "windows/mf_decoder.rs"]
mod mf_decoder;
#[cfg(windows)]
#[path = "windows/mf_encoder.rs"]
mod mf_encoder;
#[cfg(windows)]
#[path = "windows/mf_encoder_sync.rs"]
mod mf_encoder_sync;
#[cfg(windows)]
#[path = "windows/mf_media.rs"]
mod mf_media;
#[cfg(windows)]
#[path = "windows/monitors.rs"]
mod monitors;
#[cfg(target_os = "linux")]
#[path = "linux_os/pipewire_abi.rs"]
mod pipewire_abi;
#[cfg(target_os = "linux")]
#[path = "linux_os/pipewire_stream.rs"]
mod pipewire_stream;
#[cfg(target_os = "linux")]
#[path = "linux_os/portal_input.rs"]
mod portal_input;
#[cfg(target_os = "linux")]
#[path = "linux_os/portal_session.rs"]
mod portal_session;
#[cfg(target_os = "linux")]
#[path = "linux_os/portal_tokens.rs"]
pub(crate) mod portal_tokens;
#[cfg(windows)]
#[path = "windows/secure_desktop.rs"]
mod secure_desktop;
/// Ordinary window operations never cross a locked or disconnected desktop.
#[cfg(windows)]
pub(crate) fn window_control_block() -> Option<crate::screen::core::wire_control::ControlBlock> {
    use crate::screen::core::wire_control::ControlBlock;
    if !secure_desktop::session_active() {
        Some(ControlBlock::ControlUnavailable)
    } else if secure_desktop::on_secure_desktop() {
        Some(ControlBlock::SecureDesktop)
    } else {
        None
    }
}
#[cfg(target_os = "linux")]
#[path = "linux_os/session_detect.rs"]
pub(crate) mod session_detect;
#[cfg(target_os = "linux")]
#[path = "linux_os/spa_pod.rs"]
mod spa_pod;
#[cfg(target_os = "linux")]
#[path = "linux_os/wayland_toplevels.rs"]
mod wayland_toplevels;
#[cfg(windows)]
#[path = "windows/window_list.rs"]
mod window_list;
#[cfg(target_os = "linux")]
#[path = "linux_os/x11_capture.rs"]
mod x11_capture;
#[cfg(target_os = "linux")]
#[path = "linux_os/x11_cursor.rs"]
mod x11_cursor;
#[cfg(target_os = "linux")]
#[path = "linux_os/x11_input.rs"]
mod x11_input;
#[cfg(target_os = "linux")]
#[path = "linux_os/x11_keymap.rs"]
mod x11_keymap;
#[cfg(target_os = "linux")]
#[path = "linux_os/x11_monitors.rs"]
mod x11_monitors;
#[cfg(target_os = "linux")]
#[path = "linux_os/x11_windows.rs"]
mod x11_windows;

/// Whether a window manager manages the X server of `DISPLAY` (EWMH).
#[cfg(target_os = "linux")]
pub(crate) fn x11_window_manager_running() -> std::io::Result<bool> {
    x11_windows::window_manager_running()
}

/// Task tests of the Windows host with real windows and monitors (stage
/// `windows-screen`).
#[cfg(all(windows, test))]
#[path = "windows/capture_tests.rs"]
mod capture_tests;
/// Static checks of the Windows sources, run on every platform.
#[cfg(test)]
#[path = "shared/windows_static_tests.rs"]
mod windows_static_tests;

// SCR-M8 (U27): Android as host. The app captures and encodes; these files
// keep the session and the bridge to it. Plain Rust, so the host tests of
// every platform check them too.
#[cfg(any(target_os = "android", test))]
#[path = "android/capture_admission.rs"]
mod android_capture_admission;
#[cfg(any(target_os = "android", test))]
#[path = "android/capture_arena.rs"]
pub(crate) mod android_capture_arena;
#[cfg(all(target_os = "android", feature = "android-instrumentation-fixtures"))]
#[path = "android/capture_fixture.rs"]
pub(crate) mod android_capture_fixture;
#[cfg(any(target_os = "android", test))]
#[path = "android/capture_prepared.rs"]
mod android_capture_prepared;
#[cfg(any(target_os = "android", test))]
#[path = "android/bridge_backend.rs"]
pub(crate) mod bridge_backend;
#[cfg(any(target_os = "android", test))]
#[path = "android/bridge_channels.rs"]
pub(crate) mod bridge_channels;
#[cfg(any(target_os = "android", test))]
#[path = "android/bridge_source.rs"]
mod bridge_source;
#[cfg(test)]
#[path = "android/bridge_tests.rs"]
mod bridge_tests;
// SCR-M9 (U28): Android as viewer; the queue behind `screenPull`.
#[cfg(any(target_os = "android", test))]
#[path = "android/viewer_sink.rs"]
pub(crate) mod viewer_sink;
#[cfg(test)]
#[path = "android/viewer_sink_tests.rs"]
mod viewer_sink_tests;

/// Encoder and decoder factories of one kind of codec.
type Factories = (Vec<Arc<dyn EncoderFactory>>, Vec<Arc<dyn DecoderFactory>>);

/// The backend that captures on this platform: Windows (SCR-M4) and Linux
/// (SCR-M5) in the host process, Android (SCR-M8) in the app; the test
/// picture on every other platform. Share-node tests take the test picture
/// themselves (`share::node_screen`).
#[cfg(target_os = "linux")]
fn platform_backend() -> Arc<dyn ScreenHostBackend> {
    Arc::new(host_backend::LinuxBackend::new())
}

#[cfg(windows)]
fn platform_backend() -> Arc<dyn ScreenHostBackend> {
    Arc::new(host_backend::WindowsBackend::new())
}

#[cfg(target_os = "android")]
fn platform_backend() -> Arc<dyn ScreenHostBackend> {
    Arc::new(bridge_backend::BridgeBackend)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "android")))]
fn platform_backend() -> Arc<dyn ScreenHostBackend> {
    Arc::new(synthetic_backend::SyntheticBackend)
}

/// The backend of the share host: on desktops capture and encoding in the
/// host process, everything else answered by the platform backend.
#[cfg(not(target_os = "android"))]
pub(crate) fn host_backend() -> Arc<dyn ScreenHostBackend> {
    Arc::new(host_worker_client::WorkerBackend::new(platform_backend()))
}

/// The backend of the share host on Android: the app is the capture process,
/// so there is no host process.
#[cfg(target_os = "android")]
pub(crate) fn host_backend() -> Arc<dyn ScreenHostBackend> {
    platform_backend()
}

/// The host user's sign of running sessions (C7): the indicator window on
/// desktops, the app's notification and status bar chip on Android. On
/// desktops the host backend also learns the registry there, whose sessions
/// name the mode a host process opens for (input, SCR-M6).
#[cfg(not(target_os = "android"))]
pub(crate) fn host_indicator(registry: &Arc<ScreenRegistry>) -> Arc<dyn SessionIndicator> {
    host_worker_client::watch_registry(registry);
    Arc::new(indicator::DesktopIndicator::new(registry))
}

/// The source a host process opened, where its input points (SCR-M6); the
/// Wayland portal session keeps its own.
#[cfg(windows)]
fn note_input_target(source: &crate::screen::core::window_list::SourceId) {
    input_send::set_target(source);
}

#[cfg(target_os = "linux")]
fn note_input_target(source: &crate::screen::core::window_list::SourceId) {
    x11_input::set_target(source);
}

#[cfg(not(any(windows, target_os = "linux")))]
fn note_input_target(_source: &crate::screen::core::window_list::SourceId) {}

#[cfg(target_os = "android")]
pub(crate) fn host_indicator(registry: &Arc<ScreenRegistry>) -> Arc<dyn SessionIndicator> {
    Arc::new(bridge_backend::BridgeIndicator::new(registry))
}

#[path = "codec_facade.rs"]
mod codec_facade;
#[cfg(test)]
use codec_facade::media_foundation_present;
use codec_facade::{media_foundation_codecs, openh264_codecs, openh264_switched_on};
pub(crate) use codec_facade::{openh264_action, openh264_license, OpenH264Action};
