//! Recovery has its own explicit destination and remains discoverable after
//! the active mount disappears. Vault recovery always preserves sealed bytes.
use crate::app::prelude::egui;

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

/// The recovery whose deletion the user asked for: (recovery ID, drive label).
pub(super) fn show_retained(
    ui: &mut egui::Ui,
    records: &[crate::mount::InactiveMountRecovery],
) -> Option<(String, String)> {
    let mut discard = None;
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
                if ui.button("Gecachte Änderungen löschen").clicked() {
                    discard = Some((record.id.clone(), record.config.label.clone()));
                }
            });
    }
    discard
}

/// A final deletion of cached drive changes the user still confirms (user
/// 2026-10-27: „ja löschen können will ich es trotzdem“).
pub(super) enum Discard {
    /// A drive with cached changes: entry removed, its changes deleted.
    Drive { id: crate::mount::MountId, label: String },
    /// An inactive recovery: the cache it retains deleted.
    Recovery { id: String, label: String },
}

impl App {
    pub(super) fn ui_mount_discard(&mut self, ctx: &egui::Context) {
        let Some(discard) = self.mount_ui.discard.as_ref() else {
            return;
        };
        let (label, drive) = match discard {
            Discard::Drive { label, .. } => (label.clone(), true),
            Discard::Recovery { label, .. } => (label.clone(), false),
        };
        let mut open = true;
        let mut confirm = false;
        let idle = self.mount_ui.action_rx.is_none();
        super::super::dialog_geometry::window(ctx, "mount_discard", "Gecachte Änderungen löschen")
            .open(&mut open).default_width(480.0)
            .max_size(super::super::theme::window_content_limit(ctx))
            .show(ctx, |ui| {
                ui.label(format!("Laufwerk: {label}"));
                ui.label("Die lokal gecachten, noch nicht übertragenen Änderungen werden endgültig gelöscht. Dateien auf dem Remote bleiben unverändert.");
                if drive {
                    ui.label("Der Laufwerkseintrag wird entfernt; das Laufwerk lässt sich danach neu einrichten.");
                }
                confirm = ui
                    .add_enabled(idle, egui::Button::new("Endgültig löschen"))
                    .clicked();
            });
        if confirm {
            match self.mount_ui.discard.take() {
                Some(Discard::Drive { id, label }) => {
                    self.spawn_mount_action("Gecachte Änderungen werden gelöscht", move || {
                        discard_drive(id, label)
                    });
                }
                Some(Discard::Recovery { id, label }) => {
                    self.spawn_mount_action("Gecachte Änderungen werden gelöscht", move || {
                        crate::daemon::discard_mount_recovery(id.clone()).map(|()| {
                            MountUiResult::Discarded {
                                mount: None,
                                recovery: Some(id),
                                label,
                            }
                        })
                    });
                }
                None => {}
            }
        } else if !open {
            self.mount_ui.discard = None;
        }
    }
}

/// Removes a drive with cached changes through the checked save-and-remove
/// transaction into a private temporary folder, then deletes the retained
/// cache and that folder.
fn discard_drive(id: crate::mount::MountId, label: String) -> Result<MountUiResult, String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let staging = std::env::temp_dir().join(format!("se-drive-discard-{}-{stamp}", id.as_str()));
    std::fs::create_dir(&staging).map_err(|error| error.to_string())?;
    let result = crate::daemon::save_remove_mount(id, staging.clone()).and_then(|record| {
        crate::daemon::discard_mount_recovery(record.id.clone()).map(|()| record)
    });
    let _ = std::fs::remove_dir_all(&staging);
    let record = result?;
    Ok(MountUiResult::Discarded {
        mount: Some(record.config.id),
        recovery: Some(record.id),
        label,
    })
}
