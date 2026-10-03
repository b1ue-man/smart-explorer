//! Keep the window and update handoff alive until actual sync workers end.
use std::time::{Duration, Instant};

use super::prelude::*;
use super::*;

const SYNC_EXIT_WAIT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum ExitIntent {
    Close,
    Update,
}

#[derive(Default)]
pub(in crate::app) struct SyncExitGate {
    intent: Option<ExitIntent>,
    started: Option<Instant>,
}

impl App {
    pub(in crate::app) fn request_sync_exit(&mut self, intent: ExitIntent) {
        self.sync_exit_gate.intent = Some(intent);
    }

    /// Called after ordinary result drains, before admitting new UI work.
    pub(in crate::app) fn sync_exit_gate_frame(&mut self, ctx: &egui::Context) -> bool {
        self.drain_desktop_sync_workers();
        let active = self.desktop_sync_active();
        if active && ctx.input(|input| input.viewport().close_requested()) {
            self.request_sync_exit(ExitIntent::Close);
        }
        let Some(intent) = self.sync_exit_gate.intent else {
            return false;
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        if !active {
            self.sync_exit_gate = SyncExitGate::default();
            match intent {
                ExitIntent::Close => match self.prepare_for_exit(false) {
                    Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                    Err(error) => self.error_msg = Some(error),
                },
                ExitIntent::Update => {
                    if let Some(ready) = self.update_ready.clone() {
                        self.apply_ready_update(ctx, ready);
                    } else {
                        self.error_msg = Some("Das Update ist nicht mehr bereit. Bitte erneut prüfen.".into());
                    }
                }
            }
            return true;
        }

        let waiting = self.sync_exit_gate.started.is_some();
        let overdue = self.sync_exit_gate.started.is_some_and(|started| started.elapsed() >= SYNC_EXIT_WAIT);
        let mut cancel = false;
        let mut keep_open = false;
        let mut wait_again = false;
        egui::Window::new("Synchronisierung abschließen")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                if overdue {
                    ui.label("Ein Auftrag ist noch nicht beendet. Das Fenster bleibt offen, damit Dateien und Sicherungen abgeschlossen werden können.");
                    ui.horizontal(|ui| {
                        wait_again = ui.button("Weiter warten").clicked();
                        keep_open = ui.button("Fenster offen lassen").clicked();
                    });
                } else if waiting {
                    ui.label("Der Abbruch wurde angefordert. Noch laufende Schreibschritte und ihr gespeicherter Zustand werden abgeschlossen.");
                    ui.spinner();
                    keep_open = ui.button("Fenster offen lassen").clicked();
                } else {
                    ui.label("Eine Synchronisierung, Konfliktbearbeitung, Vorschau oder Wiederherstellung läuft noch.");
                    ui.label("Sie können weiterarbeiten oder den Auftrag abbrechen und seinen Abschluss abwarten.");
                    ui.horizontal(|ui| {
                        let label = match intent {
                            ExitIntent::Close => "Abbrechen und schließen",
                            ExitIntent::Update => "Abbrechen und neu starten",
                        };
                        cancel = ui.button(label).clicked();
                        keep_open = ui.button("Weiterarbeiten").clicked();
                    });
                }
            });
        if keep_open {
            self.sync_exit_gate = SyncExitGate::default();
        } else if cancel {
            self.cancel_desktop_sync();
            self.sync_exit_gate.started = Some(Instant::now());
        } else if wait_again {
            self.sync_exit_gate.started = Some(Instant::now());
        }
        ctx.request_repaint_after(Duration::from_millis(100));
        true
    }

    /// Runs only after the sync gate or an idle explicit restart request.
    pub(in crate::app) fn apply_ready_update(&mut self, ctx: &egui::Context, ready: ReadyUpdate) {
        let preflight = match &ready {
            ReadyUpdate::Staged(bundle) => crate::updater::verify_staged_update(bundle),
            ReadyUpdate::InstalledRollback { .. } => Ok(()),
        };
        if let Err(error) = preflight {
            self.error_msg = Some(format!("Update-Staging ist nicht mehr gültig: {error}"));
            return;
        }
        if let Err(error) = self.prepare_for_update_apply() {
            self.error_msg = Some(format!(
                "Neustart wurde nicht begonnen; laufende Arbeit konnte nicht sicher bewahrt werden: {error}"
            ));
            return;
        }
        let launch = match &ready {
            ReadyUpdate::Staged(bundle) => crate::updater::apply_staged_update(bundle),
            ReadyUpdate::InstalledRollback { executable, .. } => spawn_updated_app(executable)
                .map_err(|error| format!("Rollback-Version starten: {error}")),
        };
        match launch {
            Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Err(error) => {
                self.shutdown_prepared = false;
                self.error_msg = Some(format!(
                    "Neustart-Helfer konnte nicht gestartet werden; das gestagte Update bleibt erhalten: {error}"
                ));
            }
        }
    }
}
