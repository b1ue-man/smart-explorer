//! Desktop confirmation of one unsigned update (E12, review r-lokal-2 and
//! r-lokal-3, 2026-11-07). A release of the project's feed published before
//! signing, or a plain `http://` feed without signature, is staged only after
//! the user confirms exactly that version here; nothing was downloaded
//! before. The terminal counterpart is `se update --allow-unsigned`.
use super::prelude::*;
use super::*;
use crate::app::theme;
use crate::updater::{UnsignedConsent, UnsignedFeed};

/// An unsigned update waiting for the user's decision.
#[derive(Clone, Debug)]
pub(in crate::app) struct UnsignedUpdatePrompt {
    pub(in crate::app) version: String,
    pub(in crate::app) feed: UnsignedFeed,
    /// The rollback download of one older app binary ("↩ Zurück"), not the
    /// staging of a complete update.
    pub(in crate::app) rollback: bool,
}

impl App {
    pub(in crate::app) fn ui_unsigned_update_dialog(&mut self, ctx: &egui::Context) {
        let Some(prompt) = self.unsigned_update_prompt.clone() else {
            return;
        };
        let mut confirm = false;
        let mut cancel = false;
        super::dialog_geometry::window(ctx, "dialogs.1", "Unsigniertes Update")
            .id(egui::Id::new("Unsigniertes Update"))
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let action = if prompt.rollback {
                    "Zurückrollen auf"
                } else {
                    "Update auf"
                };
                ui.label(
                    RichText::new(format!("{action} Version {} ist nicht signiert.", prompt.version))
                        .strong(),
                );
                ui.colored_label(theme::warning(ui), prompt.feed.explanation());
                ui.colored_label(
                    theme::muted(ui),
                    "Die Bestätigung gilt nur für diese eine Version; jedes weitere unsignierte Update fragt erneut.",
                );
                if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                    cancel = true;
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Abbrechen").clicked() {
                        cancel = true;
                    }
                    if ui.button("Trotzdem laden").clicked() {
                        confirm = true;
                    }
                });
            });
        if confirm {
            self.unsigned_update_prompt = None;
            self.start_unsigned_update(prompt);
        } else if cancel {
            self.unsigned_update_prompt = None;
            self.notice = Some((
                format!("Unsigniertes Update v{} nicht geladen", prompt.version),
                std::time::Instant::now(),
            ));
        }
    }

    /// Runs the refused action again with consent for exactly this version.
    fn start_unsigned_update(&mut self, prompt: UnsignedUpdatePrompt) {
        let consent = UnsignedConsent::for_version(&prompt.version);
        if prompt.rollback {
            self.start_version_download_with(prompt.version, consent);
        } else if prompt.feed == UnsignedFeed::OfficialRelease {
            // The project's GitHub release of exactly this version.
            self.start_install_download_with(prompt.version, consent);
        } else if self.update_rx.is_some() {
            self.notice = Some((
                "Eine Update-Prüfung läuft bereits…".to_string(),
                std::time::Instant::now(),
            ));
        } else {
            let (tx, rx) = unbounded();
            match crate::updater::stage_confirmed_async(tx, prompt.version) {
                Ok(()) => self.update_rx = Some(rx),
                Err(error) => {
                    self.error_msg = Some(format!("Update konnte nicht gestartet werden: {error}"));
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "update_unsigned_ui_tests.rs"]
mod tests;
