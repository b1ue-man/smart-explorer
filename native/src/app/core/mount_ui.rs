use super::mount_peer_roots::PeerMountDiscovery;
use super::mount_runtime_ui::{install_controls, present_install_outcome};
use super::mount_ui_draft::MountDraft;
use super::mount_ui_helpers::{
    has_mount_attention, mount_list_alert, mount_status_alert, recovery_label, status_label,
    upsert_mount,
};
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
    received_initial_list: bool,
}

pub(super) enum MountUiResult {
    Snapshot(crate::mount::MountSnapshot),
    List(Vec<crate::mount::MountSnapshot>),
    RuntimeInstalled(crate::mount::DriveRuntimeInstallOutcome),
    /// DRV-M3: a changed option; the draft window stays as it is.
    Options(crate::mount::MountSnapshot),
    /// DRV-M5: the drive was shown in the file manager; nothing changes.
    Opened,
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
            received_initial_list: false,
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
                        let initial = !self.mount_ui.received_initial_list;
                        self.mount_ui.received_initial_list = true;
                        if initial && has_mount_attention(&mounts) {
                            self.mount_ui.show_manager = true;
                        }
                        let alert = mount_list_alert(&self.mount_ui.mounts, &mounts, initial);
                        mounts.retain(|mount| {
                            !matches!(&mount.status, crate::mount::MountStatus::Unmounted)
                        });
                        // DRV-M6, C5: the one-time hint after the device tree
                        // came up by itself.
                        if let Some(hint) = super::mount_ui_open::devices_hint(&mounts) {
                            self.notice = Some((hint, Instant::now()));
                        }
                        self.mount_ui.mounts = mounts;
                        if let Some(alert) = alert {
                            self.error_msg = Some(alert);
                        }
                    }
                    MountUiResult::RuntimeInstalled(outcome) => {
                        present_install_outcome(self, outcome);
                    }
                    MountUiResult::Opened => {}
                    MountUiResult::Options(snapshot) => {
                        let state = if snapshot.config.auto_remount {
                            "ein"
                        } else {
                            "aus"
                        };
                        self.notice = Some((
                            format!(
                                "Laufwerk „{}“: Beim Anmelden automatisch einbinden {state}",
                                snapshot.config.label
                            ),
                            Instant::now(),
                        ));
                        upsert_mount(&mut self.mount_ui.mounts, snapshot);
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
        // DRV-M4: the folder an eject unmounts first (Linux).
        let mut eject_folder: Option<std::path::PathBuf> = None;
        let mut options: Option<(crate::mount::MountId, bool)> = None;
        // DRV-M5: „Im Dateimanager öffnen“.
        let mut open_folder: Option<std::path::PathBuf> = None;
        let mut install_runtime = false;
        egui::Window::new("Smart-Explorer-Laufwerke")
            .open(&mut open)
            .default_width(600.0)
            .max_size(theme::window_content_limit(ctx))
            .vscroll(true)
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
                        egui::CollapsingHeader::new("Laufwerkdetails")
                            .id_salt(("mount_details", &mount.config.id)).show(ui, |ui| {
                                ui.label(format!("ID: {}", mount.config.id));
                                if let Some(folder) = mount.config.location_directory() {
                                    ui.label(format!("Einhängeort: {}", folder.display()));
                                }
                                ui.label(recovery_label(mount.recovery));
                            });
                        // DRV-M3: the option of a managed drive, as `se drive set`.
                        let mut auto_remount = mount.config.auto_remount;
                        if ui
                            .add_enabled(
                                self.mount_ui.action_rx.is_none(),
                                egui::Checkbox::new(
                                    &mut auto_remount,
                                    "Beim Anmelden automatisch einbinden",
                                ),
                            )
                            .on_hover_text("Bindet das Laufwerk nach jeder Anmeldung und nach einem Neustart des Hintergrunddienstes wieder ein. Auswerfen entfernt es samt dieser Einstellung.")
                            .changed()
                        {
                            options = Some((mount.config.id.clone(), auto_remount));
                        }
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
                                if let Some(location) =
                                    super::mount_ui_open::mounted_location(&mount.status)
                                {
                                    if ui
                                        .add_enabled(
                                            self.mount_ui.action_rx.is_none(),
                                            egui::Button::new(
                                                super::mount_ui_open::open_button_label(),
                                            ),
                                        )
                                        .clicked()
                                    {
                                        open_folder = Some(location);
                                    }
                                }
                                if ui
                                    .add_enabled(
                                        self.mount_ui.action_rx.is_none(),
                                        egui::Button::new("Auswerfen"),
                                    )
                                    .clicked()
                                {
                                    action = Some((false, mount.config.id.clone()));
                                    eject_folder = mounted_folder(&mount.status);
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
        } else if let Some(location) = open_folder {
            self.open_in_file_manager(location);
        } else if let Some((id, auto_remount)) = options {
            self.spawn_mount_action("Einstellung wird gespeichert", move || {
                crate::daemon::set_mount_options(id, auto_remount).map(MountUiResult::Options)
            });
        } else if let Some((retry, id)) = action {
            if retry {
                self.spawn_mount_action("Laufwerk wird erneut verbunden", move || {
                    crate::daemon::retry_mount(id).map(MountUiResult::Snapshot)
                });
            } else {
                self.spawn_mount_action("Laufwerk wird ausgeworfen", move || {
                    eject(id, eject_folder)
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

/// The folder a Linux drive is mounted at (DRV-M4); Windows ejects by letter.
fn mounted_folder(status: &crate::mount::MountStatus) -> Option<std::path::PathBuf> {
    match status {
        crate::mount::MountStatus::MountedAt { path } if cfg!(target_os = "linux") => {
            Some(path.clone())
        }
        _ => None,
    }
}

/// „Auswerfen“. Linux (DRV-M4): the folder is unmounted first, like a file
/// manager's eject; a busy drive stays mounted and says why. The host then
/// ends like on an unmount request, possibly before the stop arrives.
fn eject(
    id: crate::mount::MountId,
    folder: Option<std::path::PathBuf>,
) -> Result<MountUiResult, String> {
    // DRV-M6: ejecting the device tree switches it off, which also forgets
    // it when unsaved changes of devices remain (their journals stay).
    if id.as_str() == crate::mount::DEVICES_MOUNT_ID {
        crate::daemon::devices_disable()?;
        return crate::daemon::list_mounts().map(MountUiResult::List);
    }
    #[cfg(target_os = "linux")]
    if let Some(folder) = folder {
        crate::mount::unmount_drive_folder(&folder, false).map_err(|error| {
            if error.kind() == std::io::ErrorKind::ResourceBusy {
                format!(
                    "Laufwerk wird noch benutzt: {} (offene Dateien oder Arbeitsverzeichnisse). Schließe sie und wirf erneut aus; im Terminal erzwingt `se drive unmount --force {id}` das Auswerfen.",
                    folder.display()
                )
            } else {
                format!("Laufwerk aushängen: {error}")
            }
        })?;
        return match crate::daemon::stop_mount(id) {
            Ok(snapshot) => Ok(MountUiResult::Snapshot(snapshot)),
            Err(_) => crate::daemon::list_mounts().map(MountUiResult::List),
        };
    }
    #[cfg(not(target_os = "linux"))]
    let _ = folder;
    crate::daemon::stop_mount(id).map(MountUiResult::Snapshot)
}
