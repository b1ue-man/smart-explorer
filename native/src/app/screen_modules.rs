//! Desktop-app files of strand S4 (screen, plan 2026-10-17 „Rechte, se
//! Desktop, Tresore“). `app/mod.rs` includes this file once; later S4
//! packages declare their app files here instead of in `app/mod.rs`.

// SCR-M2: the indicator window of `--screen-indicator` (V9 start kind
// `ScreenIndicator`, wired in `run_gui`).
#[path = "core/screen_indicator_ui.rs"]
mod screen_indicator_ui;
pub(crate) use screen_indicator_ui::run_screen_indicator;
// SCR-M2: tab 9 „Bildschirm“ of „Geräte & Freigaben“.
#[path = "core/share_screen_tab_ui.rs"]
pub(crate) mod share_screen_tab_ui;
// SCR-M3: section „Kodierung“ of the screen tab (codec list, OpenH264).
#[path = "core/share_screen_codec_ui.rs"]
pub(crate) mod share_screen_codec_ui;
// SCR-M5: forget the stored Wayland screen grant (Linux).
#[cfg(target_os = "linux")]
#[path = "core/share_screen_portal_ui.rs"]
pub(crate) mod share_screen_portal_ui;

// SCR-M7: the desktop viewer and the group view (`--screen-view`,
// `--screen-grid`, V5); the pages of „Geräte & Freigaben“ start it through
// `screen_launch`.
#[path = "core/screen_decode_thread.rs"]
mod screen_decode_thread;
#[path = "core/screen_devices.rs"]
mod screen_devices;
#[path = "core/screen_input_keys.rs"]
mod screen_input_keys;
#[path = "core/screen_input_map.rs"]
mod screen_input_map;
#[path = "core/screen_launch.rs"]
pub(crate) mod screen_launch;
pub(crate) use screen_launch::{run_screen_grid, run_screen_view};
#[path = "core/screen_pool.rs"]
mod screen_pool;
#[path = "core/screen_prefs.rs"]
mod screen_prefs;
#[path = "core/screen_presented.rs"]
mod screen_presented;
#[path = "core/screen_session.rs"]
mod screen_session;
#[path = "core/screen_render_caps.rs"]
pub(crate) mod screen_render_caps;
#[path = "core/screen_stats_overlay.rs"]
mod screen_stats_overlay;
#[path = "core/screen_target.rs"]
pub(crate) mod screen_target;
#[path = "core/screen_texts.rs"]
mod screen_texts;
#[path = "core/screen_texture_cpu.rs"]
mod screen_texture_cpu;
#[path = "core/screen_tile_face.rs"]
mod screen_tile_face;
#[path = "core/screen_tile_grid.rs"]
mod screen_tile_grid;
#[path = "core/screen_toolbar_ui.rs"]
mod screen_toolbar_ui;
#[path = "core/screen_view_widget.rs"]
mod screen_view_widget;
#[path = "core/screen_visibility.rs"]
mod screen_visibility;
#[path = "core/screen_scaling.rs"]
mod screen_scaling;
#[path = "core/screen_icons.rs"]
mod screen_icons;
#[path = "core/screen_icon_requests.rs"]
mod screen_icon_requests;
#[path = "core/screen_viewport.rs"]
mod screen_viewport;
#[path = "os/shared/screen_visibility.rs"]
mod screen_visibility_os;
#[path = "core/screen_viewer_app.rs"]
mod screen_viewer_app;
#[path = "core/screen_viewer_view.rs"]
mod screen_viewer_view;
#[path = "core/screen_wgpu_yuv.rs"]
mod screen_wgpu_yuv;

#[cfg(test)]
#[path = "core/screen_viewer_fake.rs"]
mod screen_viewer_fake;
#[cfg(test)]
#[path = "core/screen_viewer_misc_tests.rs"]
mod screen_viewer_misc_tests;
#[cfg(test)]
#[path = "core/screen_viewer_tests.rs"]
mod screen_viewer_tests;

#[path="core/screen_render_upload.rs"] mod screen_render_upload;
#[path="core/screen_render_owner.rs"] mod screen_render_owner;
#[path="core/screen_render_wgpu.rs"] mod screen_render_wgpu;
#[path="core/screen_render_glow.rs"] mod screen_render_glow;
#[path="core/screen_render_yuv.rs"] mod screen_render_yuv;
#[path="core/screen_render_app.rs"] pub(crate) mod screen_render_app;
#[path="core/screen_texture_worker.rs"] mod screen_texture_worker;
#[path="core/screen_session_backend.rs"] mod screen_session_backend;
#[path="core/screen_decode_run.rs"] mod screen_decode_run;

#[path="core/screen_render_cpu.rs"] mod screen_render_cpu;

#[path="core/screen_view_widget_render.rs"] mod screen_view_widget_render;
