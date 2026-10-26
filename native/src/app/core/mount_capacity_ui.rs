//! Rendering only; provider and host requests belong to their background paths.
pub(super) fn show(ui: &mut egui::Ui, mount: &crate::mount::MountSnapshot) {
    if let Some(capacity) = &mount.capacity {
        for line in capacity.display_lines() {
            ui.label(line);
        }
    } else {
        ui.label("Remote verfügbar: unbekannt (kein Hostbericht)");
        ui.label("Lokaler Zwischenspeicher verfügbar: unbekannt (kein Hostbericht)");
    }
}
