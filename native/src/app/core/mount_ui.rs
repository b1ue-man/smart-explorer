use super::mount_peer_roots::PeerMountDiscovery;
use super::mount_runtime_ui::{install_controls, present_install_outcome};
use super::mount_ui_draft::MountDraft;
use super::mount_ui_helpers::{mount_status_alert, recovery_label, status_label, upsert_mount};
use super::prelude::*;
use super::*;

const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

pub(super) struct MountUiState {
    pub(super) draft: Option<MountDraft>,
    pub(super) show_manager: bool,
    pub(super) mounts: Vec<crate::mount::MountSnapshot>,
    pub(super) action_rx: Option<Receiver<Result<MountUiResult, String>>>,
    pub(super) peer_discovery_rx: Option<Receiver<Result<PeerMountDiscovery, String>>>,
    pub(super) busy: Option<String>,
    pub(super) next_poll: Instant,
}

pub(super) enum MountUiResult {
    Snapshot(crate::mount::MountSnapshot),
    List(Vec<crate::mount::MountSnapshot>),
    RuntimeInstalled(crate::mount::DriveRuntimeInstallOutcome),
}

impl Default for MountUiState {
    fn default() -> Self {
        Self {
            draft: None,
            show_manager: false,
            mounts: Vec::new(),
            action_rx: None,
            peer_discovery_rx: None,
            busy: None,
            next_poll: Instant::now(),
        }
    }
}

impl App {
    pub(in crate::app) fn open_mount_manager(&mut self) {
        self.mount_ui.show_manager = true;
        self.mount_ui.next_poll = Instant::now();
    }

    pub(in crate::app) fn drain_mount_ui(&mut self, ctx: &egui::Context) {
        let peer_discovery = self
            .mount_ui
            .peer_discovery_rx
            .as_ref()
            .map(|receiver| receiver.try_recv());
        match peer_discovery {
            Some(Ok(Ok(discovery))) => {
                self.mount_ui.peer_discovery_rx = None;
                self.open_peer_mount_draft(discovery);
                ctx.request_repaint();
            }
            Some(Ok(Err(error))) => {
                self.mount_ui.peer_discovery_rx = None;
                self.error_msg = Some(format!("Laufwerk: {error}"));
                ctx.request_repaint();
            }
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.mount_ui.peer_discovery_rx = None;
                self.error_msg = Some("Peer-Freigabepruefung wurde ohne Ergebnis beendet".into());
            }
            Some(Err(crossbeam_channel::TryRecvError::Empty)) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
            }
            None => {}
        }

        let received = self
            .mount_ui
            .action_rx
            .as_ref()
            .map(|receiver| receiver.try_recv());
        match received {
            Some(Ok(Ok(result))) => {
                self.mount_ui.action_rx = None;
                self.mount_ui.busy = None;
                match result {
                    MountUiResult::Snapshot(snapshot) => {
                        let alert = mount_status_alert(
                            self.mount_ui
                                .mounts
                                .iter()
                                .find(|mount| mount.config.id == snapshot.config.id)
                                .map(|mount| &mount.status),
                            &snapshot,
                        );
                        upsert_mount(&mut self.mount_ui.mounts, snapshot.clone());
                        self.mount_ui.draft = None;
                        self.mount_ui.show_manager = true;
                        self.notice = Some((
                            format!("Laufwerk: {}", status_label(&snapshot.status)),
                            Instant::now(),
                        ));
                        if let Some(alert) = alert {
                            self.error_msg = Some(alert);
                        }
                    }
                    MountUiResult::List(mut mounts) => {
                        let alert = mounts.iter().find_map(|mount| {
                            let previous = self
                                .mount_ui
                                .mounts
                                .iter()
                                .find(|known| known.config.id == mount.config.id)
                                .map(|known| &known.status);
                            mount_status_alert(previous, mount)
                        });
                        mounts.retain(|mount| {
                            !matches!(&mount.status, crate::mount::MountStatus::Unmounted)
                        });
                        self.mount_ui.mounts = mounts;
                        if let Some(alert) = alert {
                            self.error_msg = Some(alert);
                        }
                    }
                    MountUiResult::RuntimeInstalled(outcome) => {
                        present_install_outcome(self, outcome);
                    }
                }
                self.mount_ui.next_poll = Instant::now() + POLL_INTERVAL;
                ctx.request_repaint();
            }
            Some(Ok(Err(error))) => {
                self.mount_ui.action_rx = None;
                self.mount_ui.busy = None;
                self.error_msg = Some(format!("Laufwerk: {error}"));
                self.mount_ui.next_poll = Instant::now() + POLL_INTERVAL;
                ctx.request_repaint();
            }
            Some(Err(crossbeam_channel::TryRecvError::Disconnected)) => {
                self.mount_ui.action_rx = None;
                self.mount_ui.busy = None;
                self.error_msg = Some("Laufwerksaktion wurde ohne Ergebnis beendet".into());
            }
            Some(Err(crossbeam_channel::TryRecvError::Empty)) | None => {}
        }

        if crate::mount::drive_mount_supported()
            && self.mount_ui.action_rx.is_none()
            && Instant::now() >= self.mount_ui.next_poll
        {
            self.spawn_mount_action("Status wird aktualisiert", || {
                crate::daemon::list_mounts().map(MountUiResult::List)
            });
        }
        if self.mount_ui.action_rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
    }

    pub(in crate::app) fn ui_mount_windows(&mut self, ctx: &egui::Context) {
        self.ui_mount_draft(ctx);
        self.ui_mount_manager(ctx);
    }

    fn ui_mount_manager(&mut self, ctx: &egui::Context) {
        if !self.mount_ui.show_manager {
            return;
        }
        let mut open = true;
        let mut action: Option<(bool, crate::mount::MountId)> = None;
        let mut install_runtime = false;
        egui::Window::new("Smart-Explorer-Laufwerke")
            .open(&mut open)
            .default_width(560.0)
            .show(ctx, |ui| {
                if let Some(busy) = &self.mount_ui.busy {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new());
                        ui.label(busy);
                    });
                }
                if self.mount_ui.mounts.is_empty() {
                    ui.label("Keine Laufwerke in dieser Smart-Explorer-Sitzung.");
                }
                for mount in &self.mount_ui.mounts {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&mount.config.label).strong());
                            ui.label(status_label(&mount.status));
                        });
                        ui.label(format!("ID: {}", mount.config.id));
                        ui.label(recovery_label(mount.recovery));
                        ui.horizontal(|ui| match &mount.status {
                            crate::mount::MountStatus::Failed { .. }
                            | crate::mount::MountStatus::RuntimeUnavailable { .. } => {
                                if ui
                                    .add_enabled(
                                        self.mount_ui.action_rx.is_none(),
                                        egui::Button::new("Erneut versuchen"),
                                    )
                                    .clicked()
                                {
                                    action = Some((true, mount.config.id.clone()));
                                }
                                if mount.recovery.is_clean()
                                    && ui
                                        .add_enabled(
                                            self.mount_ui.action_rx.is_none(),
                                            egui::Button::new("Entfernen"),
                                        )
                                        .on_hover_text(
                                            "Sauberen fehlgeschlagenen Mount aus der Verwaltung entfernen",
                                        )
                                        .clicked()
                                {
                                    action = Some((false, mount.config.id.clone()));
                                }
                            }
                            crate::mount::MountStatus::Unmounted => {}
                            _ => {
                                if ui
                                    .add_enabled(
                                        self.mount_ui.action_rx.is_none(),
                                        egui::Button::new("Auswerfen"),
                                    )
                                    .clicked()
                                {
                                    action = Some((false, mount.config.id.clone()));
                                }
                            }
                        });
                    });
                }
                ui.separator();
                let enabled = self.mount_ui.action_rx.is_none();
                ui.horizontal(|ui| {
                    install_runtime = install_controls(ui, enabled, "Dokany-Laufzeit / Hilfe");
                });
            });
        self.mount_ui.show_manager = open;
        if install_runtime {
            self.spawn_mount_action("Dokany wird sicher installiert", || {
                crate::mount::install_drive_runtime(None).map(MountUiResult::RuntimeInstalled)
            });
        } else if let Some((retry, id)) = action {
            if retry {
                self.spawn_mount_action("Laufwerk wird erneut verbunden", move || {
                    crate::daemon::retry_mount(id).map(MountUiResult::Snapshot)
                });
            } else {
                self.spawn_mount_action("Laufwerk wird ausgeworfen", move || {
                    crate::daemon::stop_mount(id).map(MountUiResult::Snapshot)
                });
            }
        }
    }

    pub(super) fn spawn_mount_action(
        &mut self,
        label: impl Into<String>,
        action: impl FnOnce() -> Result<MountUiResult, String> + Send + 'static,
    ) {
        if self.mount_ui.action_rx.is_some() {
            return;
        }
        let (sender, receiver) = unbounded();
        self.mount_ui.busy = Some(label.into());
        self.mount_ui.action_rx = Some(receiver);
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(action))
                .map_err(|_| "Laufwerksaktion wurde unerwartet beendet".to_string())
                .and_then(|result| result);
            let _ = sender.send(result);
        });
    }
}
