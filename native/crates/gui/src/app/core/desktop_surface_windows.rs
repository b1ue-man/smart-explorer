//! Dialogs of both surface hosts, after the common central panel.
use eframe::egui;

pub(super) fn show(ctx: &egui::Context) {
    crate::app::share_apps_ui::surface_windows(ctx);
    // The asynchronous P4 resolver reports into this surface's context.
    crate::app::family_picker::notifications(ctx);
}
