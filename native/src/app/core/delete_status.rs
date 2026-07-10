use super::{DeleteKind, DeleteOrigin, DeletePhase, DeleteProgress};

pub(super) fn ui_delete_progress(
    ui: &mut eframe::egui::Ui,
    progress: &DeleteProgress,
    canceling: bool,
) -> bool {
    let phase = match (progress.origin, progress.kind, progress.phase) {
        (DeleteOrigin::Recovery, _, DeletePhase::Planning) => "Recovery prüfen",
        (DeleteOrigin::Recovery, _, DeletePhase::Applying) => "Recovery bereinigen",
        (DeleteOrigin::Reclaim, _, DeletePhase::Planning) => "Reclaim-Löschen planen",
        (DeleteOrigin::Reclaim, _, DeletePhase::Applying) => "Reclaim-Papierkorb",
        (_, DeleteKind::Recycle, DeletePhase::Planning) => "Papierkorb planen",
        (_, DeleteKind::Recycle, DeletePhase::Applying) => "Papierkorb",
        (_, DeleteKind::Permanent, DeletePhase::Planning) => "Löschen planen",
        (_, DeleteKind::Permanent, DeletePhase::Applying) => "Löschen",
    };
    let current = progress
        .current_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(&progress.current_path);
    let detail = format!(
        "{phase}: {}/{} Ziele · {}/{} Einträge · {current}",
        progress.targets_processed,
        progress.targets_total,
        progress.entries_deleted,
        progress.entries_planned,
    );
    ui.label(
        eframe::egui::RichText::new(detail)
            .small()
            .color(eframe::egui::Color32::from_gray(160)),
    );
    if canceling {
        ui.colored_label(
            eframe::egui::Color32::from_rgb(230, 190, 90),
            "Abbruch läuft…",
        );
        false
    } else {
        ui.add(eframe::egui::Button::new("Löschen abbrechen").small())
            .clicked()
    }
}
