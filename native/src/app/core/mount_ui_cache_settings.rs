//! Per-drive disposable cache and runtime compatibility controls.

use eframe::egui;

pub(super) fn render_cache_settings(
    ui: &mut egui::Ui,
    cache_mib: &mut u32,
    system_runtime: &mut bool,
) {
    ui.horizontal(|ui| {
        ui.label("Datei-Cache pro Laufwerk:");
        ui.add(
            egui::DragValue::new(cache_mib)
                .range(0..=crate::mount::MAX_MOUNT_CACHE_MIB)
                .suffix(" MiB"),
        )
        .on_hover_text("Wert anklicken und eingeben. 0 deaktiviert die Aufbewahrung nach dem Schliessen.");
    });
    ui.label("Nur geschlossene, unveraenderte Dateien zaehlen zum Limit.");
    ui.label("Offene Dateien und ungespeicherte/noch nicht uebertragene Aenderungen sind ausgenommen.");
    ui.label("Bei knappem Speicher wird entbehrlicher Cache automatisch freigegeben.");
    ui.checkbox(system_runtime, "Kompatibilitaetsmodus: offizielle System-Dokany-DLL")
        .on_hover_text("Umgeht die optimierte private Laufzeit. Der offizielle Treiber und die Systeminstallation bleiben unveraendert.");
}
