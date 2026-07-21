use super::mount_ui_helpers::{
    bounded_label, drive_selection_label, mount_status_alert, status_label, upsert_mount,
};
use super::prelude::*;
use super::*;

const DOKANY_RELEASE_URL: &str = "https://github.com/dokan-dev/dokany/releases/tag/v2.3.1.1000";
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

pub(super) struct MountUiState {
    draft: Option<MountDraft>,
    show_manager: bool,
    mounts: Vec<crate::mount::MountSnapshot>,
    action_rx: Option<Receiver<Result<MountUiResult, String>>>,
    busy: Option<String>,
    next_poll: Instant,
}

struct MountDraft {
    source: crate::mount::MountSource,
    source_label: String,
    volume_label: String,
    drive: crate::mount::DriveSelection,
    read_write: bool,
    trust_remote_root: bool,
}

enum MountUiResult {
    Snapshot(crate::mount::MountSnapshot),
    List(Vec<crate::mount::MountSnapshot>),
}

impl Default for MountUiState {
    fn default() -> Self {
        Self {
            draft: None,
            show_manager: false,
            mounts: Vec::new(),
            action_rx: None,
            busy: None,
            next_poll: Instant::now(),
        }
    }
}

impl App {
    pub(in crate::app) fn offer_mount_saved(&mut self, connection: &crate::creds::SavedConnection) {
        let root = match mount_root(&connection.root) {
            Ok(root) => root,
            Err(error) => {
                self.error_msg = Some(format!("Laufwerkswurzel: {error}"));
                return;
            }
        };
        self.open_mount_draft(
            crate::mount::MountSource::SavedRemote {
                account: connection.account(),
                root,
            },
            connection.display(),
        );
    }

    pub(in crate::app) fn offer_mount_gdrive(&mut self) {
        let Ok(root) = crate::mount::BackendRoot::parse("/") else {
            self.error_msg = Some("Google-Drive-Wurzel ist ungueltig".into());
            return;
        };
        self.open_mount_draft(
            crate::mount::MountSource::GoogleDrive {
                account: "cloud:gdrive".into(),
                root,
            },
            "Google Drive".into(),
        );
    }

    pub(in crate::app) fn offer_mount_peer(
        &mut self,
        target: crate::share::PeerOpenTarget,
        label: String,
    ) {
        let target = match target {
            crate::share::PeerOpenTarget::Direct { contact_id } => {
                crate::mount::PeerMountTarget::Direct { contact_id }
            }
            crate::share::PeerOpenTarget::RoomDevice { room_id, device_id } => {
                crate::mount::PeerMountTarget::RoomDevice { room_id, device_id }
            }
        };
        let Ok(root) = crate::mount::BackendRoot::parse("/") else {
            self.error_msg = Some("Share-Wurzel ist ungueltig".into());
            return;
        };
        self.open_mount_draft(crate::mount::MountSource::Peer { target, root }, label);
    }

    pub(in crate::app) fn open_mount_manager(&mut self) {
        self.mount_ui.show_manager = true;
        self.mount_ui.next_poll = Instant::now();
    }

    fn open_mount_draft(&mut self, source: crate::mount::MountSource, label: String) {
        // Explorer exposes the volume label outside Smart Explorer. Do not use
        // endpoint, account, root, room, or contact text as its default.
        let volume_label = "Smart Explorer".to_string();
        self.mount_ui.draft = Some(MountDraft {
            source,
            source_label: label,
            volume_label,
            drive: crate::mount::DriveSelection::Automatic,
            read_write: false,
            trust_remote_root: false,
        });
    }

    pub(in crate::app) fn drain_mount_ui(&mut self, ctx: &egui::Context) {
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

    fn ui_mount_draft(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.mount_ui.draft.take() else {
            return;
        };
        let mut open = true;
        let mut cancel = false;
        let mut submit = false;
        egui::Window::new("Remote als Laufwerk")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(RichText::new(&draft.source_label).strong());
                ui.label(format!("Remote-Wurzel: {}", draft.source.root().as_str()));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label("Laufwerksbuchstabe:");
                    egui::ComboBox::from_id_salt("mount_drive_letter")
                        .selected_text(drive_selection_label(draft.drive))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut draft.drive,
                                crate::mount::DriveSelection::Automatic,
                                "Automatisch",
                            );
                            for letter in ('D'..='Z')
                                .filter_map(|letter| crate::mount::DriveLetter::parse(letter).ok())
                            {
                                ui.selectable_value(
                                    &mut draft.drive,
                                    crate::mount::DriveSelection::Letter(letter),
                                    letter.to_string(),
                                );
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    ui.text_edit_singleline(&mut draft.volume_label);
                });
                ui.checkbox(&mut draft.read_write, "Schreibzugriff erlauben");
                if draft.read_write {
                    ui.colored_label(
                        Color32::from_rgb(255, 190, 90),
                        "Editor-Saves werden als ganze Datei konfliktgeprueft ueber Smart Explorer hochgeladen.",
                    );
                    ui.label(
                        "Das Remote-Backend muss sichere Datei-Promotionen unterstuetzen; andernfalls bleibt die Aenderung im Recovery-Cache.",
                    );
                } else {
                    ui.label("Standard: schreibgeschuetzt.");
                }
                ui.checkbox(
                    &mut draft.trust_remote_root,
                    "Remote-Wurzel ohne technische Sandbox vertrauen",
                )
                .on_hover_text(
                    "Nur aktivieren, wenn Server und andere Writer vertrauenswuerdig sind. Smart Explorer prueft Pfade weiterhin, kann Protokollzugriffe dann aber nicht atomar gegen Symlink-/Junction-Rennen absichern.",
                );
                if draft.trust_remote_root {
                    ui.colored_label(
                        Color32::from_rgb(255, 150, 90),
                        "Vertrauensmodus: Ein anderer Writer am Remote kann die ausgewaehlte Wurzel waehrend eines Pfadzugriffs veraendern.",
                    );
                }
                ui.separator();
                ui.label("Voraussetzung: offizielle Dokany-2.3.1-Laufzeit (kein Entwicklermodus).");
                ui.hyperlink_to("Dokany 2.3.1 herunterladen", DOKANY_RELEASE_URL);
                if let Some(busy) = &self.mount_ui.busy {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new());
                        ui.label(busy);
                    });
                }
                ui.horizontal(|ui| {
                    let valid =
                        !draft.volume_label.trim().is_empty() && self.mount_ui.action_rx.is_none();
                    if ui
                        .add_enabled(valid, egui::Button::new("Einbinden"))
                        .clicked()
                    {
                        submit = true;
                    }
                    if ui.button("Abbrechen").clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            open = false;
        }
        if submit {
            let mode = if draft.read_write {
                crate::mount::MountMode::ReadWrite
            } else {
                crate::mount::MountMode::ReadOnly
            };
            let root_security = if draft.trust_remote_root {
                crate::mount::MountRootSecurity::Trusted
            } else {
                crate::mount::MountRootSecurity::Enforced
            };
            let config = crate::mount::MountId::new_random()
                .and_then(|id| {
                    crate::mount::MountConfig::new(
                        id,
                        draft.source.clone(),
                        draft.drive,
                        mode,
                        bounded_label(&draft.volume_label),
                    )
                })
                .map(|config| config.with_root_security(root_security))
                .map_err(|error| error.to_string());
            match config {
                Ok(config) => {
                    // Keep the choices editable until the daemon has accepted
                    // the mount (for example it may conservatively reject RW).
                    self.mount_ui.draft = Some(draft);
                    self.spawn_mount_action("Remote wird verbunden", move || {
                        crate::daemon::start_mount(config).map(MountUiResult::Snapshot)
                    });
                }
                Err(error) => {
                    self.error_msg = Some(format!("Laufwerk: {error}"));
                    self.mount_ui.draft = Some(draft);
                }
            }
        } else if open {
            self.mount_ui.draft = Some(draft);
        }
    }

    fn ui_mount_manager(&mut self, ctx: &egui::Context) {
        if !self.mount_ui.show_manager {
            return;
        }
        let mut open = true;
        let mut action: Option<(bool, crate::mount::MountId)> = None;
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
                                if !mount.recovery_required
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
                ui.hyperlink_to("Dokany-Laufzeit / Hilfe", DOKANY_RELEASE_URL);
            });
        self.mount_ui.show_manager = open;
        if let Some((retry, id)) = action {
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

    fn spawn_mount_action(
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

fn mount_root(path: &str) -> Result<crate::mount::BackendRoot, String> {
    crate::mount::BackendRoot::parse(&path.replace('\\', "/")).map_err(|error| error.to_string())
}
