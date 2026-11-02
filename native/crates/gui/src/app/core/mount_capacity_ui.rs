//! Rendering only; provider and host requests belong to their background paths.
use crate::app::prelude::egui;
use crate::mount::MountUploadStatus;

pub(super) fn show(ui: &mut egui::Ui, mount: &crate::mount::MountSnapshot) {
    if let Some(capacity) = &mount.capacity {
        for line in capacity.display_lines() {
            ui.label(line);
        }
        for line in capacity
            .uploads
            .as_ref()
            .map(MountUploadStatus::display_lines)
            .unwrap_or_default()
        {
            ui.label(line);
        }
    } else {
        ui.label("Remote verfügbar: unbekannt (kein Hostbericht)");
        ui.label("Lokaler Zwischenspeicher verfügbar: unbekannt (kein Hostbericht)");
    }
}
