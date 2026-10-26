//! Recovery has its own explicit destination and remains discoverable after
//! the active mount disappears. Vault recovery always preserves sealed bytes.
use super::super::prelude::*;
use super::{App, MountUiResult};

pub(super) struct RecoveryDraft {
    id: crate::mount::MountId,
    label: String,
    destination: String,
}

impl RecoveryDraft {
    pub(super) fn new(id: crate::mount::MountId, label: String) -> Self {
        Self {
            id,
            label,
            destination: String::new(),
        }
    }
}

impl App {
    pub(super) fn ui_mount_recovery(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.mount_ui.recovery_draft.as_mut() else {
            return;
        };
        let mut open = true;
        let mut save = false;
        super::super::dialog_geometry::window(ctx, "mount_recovery", "Laufwerksänderungen sichern")
            .open(&mut open).default_width(560.0)
            .max_size(super::super::theme::window_content_limit(ctx))
            .show(ctx, |ui| {
                ui.label(format!("Laufwerk: {}", draft.label));
                ui.label("Das Paket sichert vorhandene lokale Dateien, Originaljournal, Baselines, Lösch- und Umbenennungszustände sowie Stageverweise. Nicht lokal vorhandene Inhalte werden ausdrücklich im Manifest genannt.");
                ui.label("Tresordaten bleiben versiegelt. Der Originalcache bleibt unter dem inaktiven Recovery-Eigentümer erhalten. Das Laufwerk wird danach nicht automatisch verbunden.");
                ui.label("Vorhandener Zielordner (absoluter Pfad):");
                ui.text_edit_singleline(&mut draft.destination);
                save = ui.add_enabled(self.mount_ui.action_rx.is_none()
                    && std::path::Path::new(&draft.destination).is_absolute(),
                    egui::Button::new("Sichern und entfernen")).clicked();
            });
        if save {
            let id = draft.id.clone();
            let destination = std::path::PathBuf::from(&draft.destination);
            self.spawn_mount_action("Lokale Recovery wird dauerhaft gesichert", move || {
                crate::daemon::save_remove_mount(id, destination).map(MountUiResult::RecoverySaved)
            });
        } else if !open {
            self.mount_ui.recovery_draft = None;
        }
    }
}

pub(super) fn show_retained(ui: &mut egui::Ui, records: &[crate::mount::InactiveMountRecovery]) {
    for record in records {
        egui::CollapsingHeader::new(format!("Recovery: {} ({})", record.config.label, record.id))
            .id_salt(("inactive_mount_recovery", &record.id))
            .show(ui, |ui| {
                ui.label(format!("Paket: {}", record.package_path.display()));
                ui.label(format!(
                    "Erhaltener Cache: {}",
                    record.retained_cache.display()
                ));
                ui.label(format!("Manifest SHA-256: {}", record.manifest_sha256));
                ui.label(format!(
                    "Hinweise zur Vollständigkeit des Manifests: {}",
                    record.gap_count
                ));
                if let Some(error) = &record.package_error {
                    ui.label(error);
                }
                ui.label(if record.sealed {
                    "Tresordaten: versiegelt"
                } else {
                    "Dateiinhalte: unverschlüsselte Sicherung"
                });
                ui.label(if record.reactivated {
                    "Paket bleibt erhalten; zugehöriger Mount wurde manuell reaktiviert."
                } else {
                    "Inaktiver Bestand. Kein automatisches Einbinden."
                });
                for gap in &record.gaps {
                    let label =
                        if gap.kind == crate::mount::RecoveryGapKind::JournalInventoryUnknown {
                            "Inventur nicht ausgewertet"
                        } else {
                            "Nicht gesichert oder nicht bestätigt"
                        };
                    ui.label(format!("{label}: {} — {}", gap.subject, gap.detail));
                }
                ui.label(format!(
                    "Manuell reaktivieren: se drive recovery reactivate {}",
                    record.id
                ));
            });
    }
}
