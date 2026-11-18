//! Analysis issue reports and their existing problem-list presentation.
use crate::analytics::ScanIssue;
use crate::app::theme;
use eframe::egui;

pub(in super::super) fn issue_report(issues: &[ScanIssue], suppressed: u64, denied: u64) -> String {
    let mut report = format!(
        "{} Leseproblem(e), davon {denied} Zugriff verweigert.\n",
        (issues.len() as u64).saturating_add(suppressed)
    );
    for issue in issues {
        report.push_str(&format!("{}: {}\n", issue.path, issue.detail));
    }
    if suppressed > 0 {
        report.push_str(&format!(
            "{suppressed} weitere Probleme sind nicht einzeln gespeichert.\n"
        ));
    }
    report
}

pub(in super::super) fn issues_ui(
    ui: &mut egui::Ui,
    issues: &[ScanIssue],
    suppressed: u64,
    denied: u64,
    notes: &[String],
) {
    // How the result was made and what it left out stand first and in
    // full-size text; the other remarks stay small.
    for note in notes.iter().filter(|note| prominent_note(note)) {
        ui.label(egui::RichText::new(note).strong());
    }
    for note in notes.iter().filter(|note| !prominent_note(note)) {
        ui.label(egui::RichText::new(note).small().color(theme::muted(ui)));
    }
    if issues.is_empty() && suppressed == 0 {
        return;
    }
    egui::CollapsingHeader::new("Leseprobleme – betroffene Pfade und Details")
        .id_salt("analytics_read_issues")
        .show(ui, |ui| {
            if ui.button("Bericht kopieren").clicked() {
                ui.ctx().copy_text(issue_report(issues, suppressed, denied));
            }
            egui::ScrollArea::vertical()
                .min_scrolled_height(0.0)
                .id_salt("analytics_read_issues_scroll")
                .max_height(super::super::dialog_geometry::remaining_height(ui, 2.0))
                .show(ui, |ui| {
                    for issue in issues {
                        ui.label(egui::RichText::new(&issue.path).strong());
                        ui.label(&issue.detail);
                        ui.separator();
                    }
                    if suppressed > 0 {
                        ui.label(format!(
                            "{suppressed} weitere Probleme nicht einzeln gespeichert."
                        ));
                    }
                });
        });
}

/// The analysis way („Analyseweg: …“, MFT or directory walk) and folders on
/// another file system that were not counted.
pub(in super::super) fn prominent_note(note: &str) -> bool {
    note.starts_with("Analyseweg:") || note.starts_with("anderes Dateisystem")
}

#[cfg(test)]
mod tests {
    #[test]
    fn analyse_task_analysis_way_and_foreign_file_systems_are_prominent() {
        for note in [
            "Analyseweg: schneller NTFS-Weg (MFT)",
            "Analyseweg: Verzeichnislauf (MFT nicht lesbar: x)",
            "anderes Dateisystem, nicht gezählt: /proc",
        ] {
            assert!(super::prominent_note(note), "{note}");
        }
        let other = "3 Datei(en) sind nur in den Ordnergroessen enthalten";
        assert!(!super::prominent_note(other));
    }
}
