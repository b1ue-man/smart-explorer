//! Desktop-app files of S5 (se Desktop) and of the shared start and update
//! contracts V8/V9. `app/mod.rs` includes this file once; later S5 packages
//! declare their app files here instead of in `app/mod.rs`.

// DSK-M3: Einstellungen → Integration → „se Desktop“ and the start pages of
// `--open-location`/`--open-page`.
#[path = "core/desktop_settings_gnome_ui.rs"]
mod desktop_settings_gnome_ui;
#[path = "core/desktop_settings_ui.rs"]
mod desktop_settings_ui;
// DSK-M3: the surface process `--desktop-surface` (view: DSK-M4).
#[path = "core/desktop_surface_app.rs"]
pub(crate) mod desktop_surface_app;
// DSK-M4: the surface view, written against egui only (eframe host here,
// layer host in DSK-M7), its handlings and the section „Darstellung“.
#[path = "core/desktop_surface_actions.rs"]
pub(crate) mod desktop_surface_actions;
// DSK-M7: the file clipboard a surface process installs (Wayland
// `wl_data_device`, X11 selection `CLIPBOARD` with DSK-M6).
#[path = "core/desktop_surface_clipboard.rs"]
pub(crate) mod desktop_surface_clipboard;
#[path = "core/desktop_surface_grid_ui.rs"]
pub(crate) mod desktop_surface_grid_ui;
#[path = "core/desktop_surface_host.rs"]
pub(crate) mod desktop_surface_host;
#[path = "core/desktop_surface_input_ui.rs"]
mod desktop_surface_input_ui;
#[path = "core/desktop_surface_menu_ui.rs"]
mod desktop_surface_menu_ui;
#[path = "core/desktop_surface_rename_ui.rs"]
mod desktop_surface_rename_ui;
#[path = "core/desktop_surface_settings_ui.rs"]
mod desktop_surface_settings_ui;
#[path = "core/desktop_surface_tile_ui.rs"]
mod desktop_surface_tile_ui;
#[path = "core/desktop_surface_view_model.rs"]
pub(crate) mod desktop_surface_view_model;
#[path = "core/desktop_surface_wallpaper_ui.rs"]
mod desktop_surface_wallpaper_ui;
// V9: start switches of the GUI program, one decision for `run_gui`.
#[path = "core/start_request.rs"]
pub(crate) mod start_request;
// V8/DSK-M2: every egui window closes on a new update-stop request.
#[path = "core/update_stop_ui.rs"]
pub(crate) mod update_stop_ui;
