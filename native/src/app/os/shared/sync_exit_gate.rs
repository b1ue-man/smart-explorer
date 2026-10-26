//! Keep the window and update handoff alive until sync and RDP owners end.
use std::time::{Duration, Instant};

use super::prelude::*;
use super::*;

const SYNC_EXIT_WAIT: Duration = Duration::from_secs(10);

#[path = "rdp_exit_gate.rs"]
mod rdp_exit_gate;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum ExitIntent {
    Close,
    Update,
}

#[derive(Default)]
pub(in crate::app) struct SyncExitGate {
    intent: Option<ExitIntent>,
    started: Option<Instant>,
    update: Option<ReadyUpdate>,
    update_verified: bool,
    rdp: rdp_exit_gate::RdpExitGate,
}

impl SyncExitGate {
    fn clear_request(&mut self) {
        self.intent = None;
        self.started = None;
        self.update = None;
        self.update_verified = false;
    }

    pub(in crate::app) fn finish_rdp(&mut self) {
        self.rdp.finish();
    }
}

impl App {
    pub(in crate::app) fn request_sync_exit(&mut self, intent: ExitIntent) {
        // One close/update request survives every drain frame. Repeated OS
        // close events must not replace an already accepted update request.
        if self.sync_exit_gate.intent.is_none() {
            self.sync_exit_gate.intent = Some(intent);
            self.sync_exit_gate.update = match intent {
                ExitIntent::Update => self.update_ready.clone(),
                ExitIntent::Close => None,
            };
        }
    }

    /// Called after ordinary result drains, before admitting new UI work.
    pub(in crate::app) fn sync_exit_gate_frame(&mut self, ctx: &egui::Context) -> bool {
        self.drain_desktop_sync_workers();
        let active = self.desktop_sync_active();
        if ctx.input(|input| input.viewport().close_requested()) {
            self.request_sync_exit(ExitIntent::Close);
        }
        let Some(intent) = self.sync_exit_gate.intent else {
            // A user can keep the window open while a newly appeared sync
            // worker is still active. Retain any drain already begun, and
            // reopen RDP admission only after that drain has actually ended.
            if let Err(error) = self.sync_exit_gate.rdp.resume(ctx) {
                self.error_msg = Some(error);
            }
            return false;
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        if !active {
            // Invalid staging must not close otherwise usable RDP sessions.
            if intent == ExitIntent::Update && !self.sync_exit_gate.update_verified {
                let preflight = self.sync_exit_gate.update.as_ref()
                    .ok_or_else(|| "Das Update ist nicht mehr bereit. Bitte erneut prüfen.".to_string())
                    .and_then(verify_ready_update);
                if let Err(error) = preflight {
                    self.error_msg = Some(error);
                    self.sync_exit_gate.clear_request();
                    return true;
                }
                self.sync_exit_gate.update_verified = true;
            }
            if !self.sync_exit_gate.rdp.poll(ctx) {
                self.ui_rdp_exit_wait(ctx);
                return true;
            }
            let result = match intent {
                ExitIntent::Close => self.prepare_for_exit(false).map(|()| {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }),
                ExitIntent::Update => match self.sync_exit_gate.update.clone() {
                    Some(ready) => self.finish_ready_update(ctx, ready),
                    None => Err("Das Update ist nicht mehr bereit. Bitte erneut prüfen.".into()),
                },
            };
            // Preparation and update launch each run only after a completed
            // RDP drain. Genuine preflight/recovery/launch errors stop retrying;
            // pending RDP never reaches this branch and keeps its intent.
            self.sync_exit_gate.clear_request();
            if let Err(error) = result {
                self.error_msg = Some(error);
                if let Err(error) = self.sync_exit_gate.rdp.resume(ctx) {
                    self.error_msg = Some(error);
                }
            }
            return true;
        }

        let waiting = self.sync_exit_gate.started.is_some();
        let overdue = self
            .sync_exit_gate
            .started
            .is_some_and(|started| started.elapsed() >= SYNC_EXIT_WAIT);
        let mut cancel = false;
        let mut keep_open = false;
        let mut wait_again = false;
        crate::app::dialog_geometry::window(ctx, "sync_exit_gate.0", "Synchronisierung abschließen")
            .id(egui::Id::new("Synchronisierung abschließen"))
            .collapsible(false)
            .resizable(true)
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
            self.sync_exit_gate.clear_request();
        } else if cancel {
            self.cancel_desktop_sync();
            self.sync_exit_gate.started = Some(Instant::now());
        } else if wait_again {
            self.sync_exit_gate.started = Some(Instant::now());
        }
        ctx.request_repaint_after(Duration::from_millis(100));
        true
    }

    fn ui_rdp_exit_wait(&self, ctx: &egui::Context) {
        crate::app::dialog_geometry::window(ctx, "sync_exit_gate.rdp", "RDP-Sitzungen schließen")
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Smart Explorer wartet, bis laufende RDP-Starts, Clientprozesse, TCP-Leases und Sitzungsmonitore beendet sind.");
                ui.label("Der Schließ- oder Neustartwunsch bleibt gespeichert. Der Abschluss wird automatisch erneut geprüft.");
                ui.spinner();
                if let Some(error) = self.sync_exit_gate.rdp.error() {
                    ui.label(error);
                }
            });
        ctx.request_repaint_after(Duration::from_millis(100));
    }

    /// Idle restart requests use exactly the same retained intent as requests
    /// made while sync is active. No helper may launch before RDP has drained.
    pub(in crate::app) fn apply_ready_update(&mut self, ctx: &egui::Context, ready: ReadyUpdate) {
        self.request_sync_exit(ExitIntent::Update);
        if self.sync_exit_gate.intent == Some(ExitIntent::Update) {
            self.sync_exit_gate.update = Some(ready);
            self.sync_exit_gate.update_verified = false;
        }
        ctx.request_repaint();
    }

    fn finish_ready_update(&mut self, ctx: &egui::Context, ready: ReadyUpdate) -> Result<(), String> {
        // The drain may have waited for a peer approval. Revalidate bytes now,
        // immediately before shutdown preparation and the existing apply API.
        verify_ready_update(&ready)?;
        self.prepare_for_update_apply().map_err(|error| format!(
                "Neustart wurde nicht begonnen; laufende Arbeit konnte nicht sicher bewahrt werden: {error}"
            ))?;
        let launch = match &ready {
            ReadyUpdate::Staged(bundle) => crate::updater::apply_staged_update(bundle),
            ReadyUpdate::InstalledRollback { executable, .. } => spawn_updated_app(executable)
                .map_err(|error| format!("Rollback-Version starten: {error}")),
        };
        match launch {
            Ok(()) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                Ok(())
            }
            Err(error) => {
                self.shutdown_prepared = false;
                Err(format!(
                    "Neustart-Helfer konnte nicht gestartet werden; das gestagte Update bleibt erhalten: {error}"
                ))
            }
        }
    }
}

fn verify_ready_update(ready: &ReadyUpdate) -> Result<(), String> {
    match ready {
        ReadyUpdate::Staged(bundle) => crate::updater::verify_staged_update(bundle),
        ReadyUpdate::InstalledRollback { .. } => Ok(()),
    }.map_err(|error| format!("Update-Staging ist nicht mehr gültig: {error}"))
}

#[cfg(test)]
#[path = "sync_exit_gate_task_tests.rs"]
mod task_tests;
