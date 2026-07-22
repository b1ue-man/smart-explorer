use super::mount_peer_roots::{
    begin_peer_probe, discover_peer_mount, poll_peer_probe, PeerDraft, PeerMountDiscovery,
};
use super::mount_runtime_ui::install_controls;
use super::mount_ui::MountUiResult;
use super::mount_ui_helpers::{bounded_label, drive_selection_label};
use super::prelude::*;
use super::*;

pub(super) struct MountDraft {
    pub(super) source: crate::mount::MountSource,
    source_label: String,
    volume_label: String,
    drive: crate::mount::DriveSelection,
    read_write: bool,
    trust_remote_root: bool,
    pub(super) peer: Option<PeerDraft>,
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
        if self.mount_ui.peer_discovery_rx.is_some() {
            self.error_msg = Some("Eine Peer-Freigabe wird bereits geprueft".into());
            return;
        }
        let (sender, receiver) = unbounded();
        self.mount_ui.peer_discovery_rx = Some(receiver);
        self.notice = Some(("Peer-Freigaben werden geprueft".into(), Instant::now()));
        if let Err(error) = std::thread::Builder::new()
            .name("peer-mount-discovery".into())
            .spawn(move || {
                let _ = sender.send(discover_peer_mount(target, label));
            })
        {
            self.mount_ui.peer_discovery_rx = None;
            self.error_msg = Some(format!("Peer-Freigabepruefung starten: {error}"));
        }
    }

    fn open_mount_draft(&mut self, source: crate::mount::MountSource, label: String) {
        self.mount_ui.draft = Some(MountDraft {
            source,
            source_label: label,
            volume_label: "Smart Explorer".into(),
            drive: crate::mount::DriveSelection::Automatic,
            read_write: false,
            trust_remote_root: false,
            peer: None,
        });
    }

    pub(super) fn open_peer_mount_draft(&mut self, discovery: PeerMountDiscovery) {
        let Some(root) = discovery
            .roots
            .get(discovery.selected)
            .map(|choice| choice.root.clone())
        else {
            self.error_msg = Some("Der Peer hat keine einbindbare Freigabe gemeldet".into());
            return;
        };
        self.mount_ui.draft = Some(MountDraft {
            source: crate::mount::MountSource::Peer {
                target: discovery.mount_target,
                root,
            },
            source_label: discovery.label,
            volume_label: "Smart Explorer".into(),
            drive: crate::mount::DriveSelection::Automatic,
            read_write: false,
            trust_remote_root: false,
            peer: Some(PeerDraft {
                open_target: discovery.open_target,
                roots: discovery.roots,
                selected: discovery.selected,
                safety: Some(discovery.safety),
                probe_rx: None,
            }),
        });
    }

    pub(super) fn ui_mount_draft(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.mount_ui.draft.take() else {
            return;
        };
        poll_peer_probe(&mut draft, ctx);
        let mut open = true;
        let mut cancel = false;
        let mut submit = false;
        let mut install_runtime = false;
        egui::Window::new("Remote als Laufwerk")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(RichText::new(&draft.source_label).strong());
                render_root_choice(ui, &mut draft, ctx);
                ui.add_space(6.0);
                render_drive_choice(ui, &mut draft);
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    ui.text_edit_singleline(&mut draft.volume_label);
                });
                let writable_root = !peer_root_is_synthetic(&draft);
                if !writable_root {
                    draft.read_write = false;
                }
                ui.add_enabled(
                    writable_root,
                    egui::Checkbox::new(&mut draft.read_write, "Schreibzugriff erlauben"),
                );
                render_write_status(ui, &draft);
                let trust_allowed = match peer_safety(&draft) {
                    Some(Ok(capabilities)) => !capabilities.root_confinement.is_enforced(),
                    Some(Err(_)) => false,
                    None if draft.peer.is_some() => false,
                    None => true,
                };
                ui.add_enabled(
                    trust_allowed,
                    egui::Checkbox::new(
                        &mut draft.trust_remote_root,
                        "Dieser ausgewaehlten Remote-Wurzel ausdruecklich vertrauen",
                    ),
                )
                .on_hover_text(
                    "Erforderlich, wenn das Remote-Protokoll Pfade zwar prueft, aber Symlink-/Junction-Rennen nicht technisch ausschliessen kann.",
                );
                render_root_security(ui, &mut draft, ctx);
                ui.separator();
                ui.label("Voraussetzung: offizielle Dokany-2.3.1-Laufzeit (kein Entwicklermodus).");
                let enabled = self.mount_ui.action_rx.is_none();
                ui.horizontal(|ui| {
                    install_runtime = install_controls(ui, enabled, "Manueller Download");
                });
                if let Some(busy) = &self.mount_ui.busy {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new());
                        ui.label(busy);
                    });
                }
                ui.horizontal(|ui| {
                    let valid = draft_ready(&draft) && self.mount_ui.action_rx.is_none();
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
        if install_runtime {
            self.mount_ui.draft = Some(draft);
            self.spawn_mount_action("Dokany wird sicher installiert", || {
                crate::mount::install_drive_runtime(None).map(MountUiResult::RuntimeInstalled)
            });
            return;
        }
        if cancel {
            open = false;
        }
        if submit {
            self.submit_mount_draft(draft);
        } else if open {
            self.mount_ui.draft = Some(draft);
        }
    }

    fn submit_mount_draft(&mut self, draft: MountDraft) {
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
    }
}

fn render_root_choice(ui: &mut egui::Ui, draft: &mut MountDraft, ctx: &egui::Context) {
    let Some(peer) = draft.peer.as_mut() else {
        ui.label(format!("Remote-Wurzel: {}", draft.source.root().as_str()));
        return;
    };
    let previous = peer.selected;
    ui.add_enabled_ui(peer.probe_rx.is_none(), |ui| {
        egui::ComboBox::from_id_salt("peer_mount_root")
            .selected_text(&peer.roots[peer.selected].label)
            .show_ui(ui, |ui| {
                for (index, choice) in peer.roots.iter().enumerate() {
                    ui.selectable_value(&mut peer.selected, index, &choice.label)
                        .on_hover_text(choice.root.as_str());
                }
            });
    });
    if peer.selected != previous {
        let root = peer.roots[peer.selected].root.clone();
        if let crate::mount::MountSource::Peer {
            root: selected_root,
            ..
        } = &mut draft.source
        {
            *selected_root = root;
        }
        draft.trust_remote_root = false;
        if peer.roots[peer.selected].root.as_str() == "/" {
            draft.read_write = false;
        }
        begin_peer_probe(peer, ctx);
    }
    ui.label(format!("Remote-Wurzel: {}", draft.source.root().as_str()));
}

fn render_drive_choice(ui: &mut egui::Ui, draft: &mut MountDraft) {
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
                for letter in
                    ('D'..='Z').filter_map(|letter| crate::mount::DriveLetter::parse(letter).ok())
                {
                    ui.selectable_value(
                        &mut draft.drive,
                        crate::mount::DriveSelection::Letter(letter),
                        letter.to_string(),
                    );
                }
            });
    });
}

fn render_write_status(ui: &mut egui::Ui, draft: &MountDraft) {
    if peer_root_is_synthetic(draft) {
        ui.label("Die zusammengefasste Peer-Wurzel ist immer schreibgeschuetzt.");
        return;
    }
    if !draft.read_write {
        ui.label("Standard: schreibgeschuetzt.");
        return;
    }
    ui.colored_label(
        Color32::from_rgb(255, 190, 90),
        "Editor-Saves werden als ganze Datei konfliktgeprueft ueber Smart Explorer hochgeladen.",
    );
    if let Some(Ok(capabilities)) = peer_safety(draft) {
        if !capabilities.staged_write.supports_mounted_writes() {
            ui.colored_label(
                Color32::from_rgb(255, 120, 100),
                format!(
                    "Diese Peer-Wurzel ist nicht sicher schreibbar: {}.",
                    missing_writes(capabilities.staged_write)
                ),
            );
        }
    }
}

fn render_root_security(ui: &mut egui::Ui, draft: &mut MountDraft, ctx: &egui::Context) {
    let safety = peer_safety(draft).cloned();
    match safety {
        Some(Ok(capabilities)) if capabilities.root_confinement.is_enforced() => {
            ui.colored_label(
                Color32::from_rgb(120, 220, 150),
                "Technische Sandbox fuer diese Wurzel bestaetigt.",
            );
        }
        Some(Ok(_)) if !draft.trust_remote_root => {
            ui.colored_label(
                Color32::from_rgb(255, 120, 100),
                "Diese Wurzel ist nur im Vertrauensmodus einbindbar. Aktiviere die ausdrueckliche Freigabe oben oder waehle ein technisch eingegrenztes Ziel.",
            );
        }
        Some(Err(error)) => {
            ui.colored_label(
                Color32::from_rgb(255, 120, 100),
                format!("Peer-Faehigkeiten konnten nicht bestaetigt werden: {error}"),
            );
            if draft
                .peer
                .as_ref()
                .is_some_and(|peer| peer.probe_rx.is_none())
                && ui.button("Erneut pruefen").clicked()
            {
                if let Some(peer) = draft.peer.as_mut() {
                    begin_peer_probe(peer, ctx);
                }
            }
        }
        None if draft.peer.is_some() => {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new());
                ui.label("Peer-Faehigkeiten werden geprueft ...");
            });
        }
        _ if draft.trust_remote_root => {
            ui.colored_label(
                Color32::from_rgb(255, 150, 90),
                "Vertrauensmodus: Ein anderer Writer am Remote kann die Wurzel waehrend eines Pfadzugriffs veraendern.",
            );
        }
        _ => {}
    }
}

fn draft_ready(draft: &MountDraft) -> bool {
    if draft.volume_label.trim().is_empty() {
        return false;
    }
    let Some(safety) = peer_safety(draft) else {
        return draft.peer.is_none();
    };
    let Ok(capabilities) = safety else {
        return false;
    };
    let root_ready = capabilities.root_confinement.is_enforced() || draft.trust_remote_root;
    let writes_ready = !draft.read_write
        || (!peer_root_is_synthetic(draft) && capabilities.staged_write.supports_mounted_writes());
    root_ready && writes_ready
}

fn peer_root_is_synthetic(draft: &MountDraft) -> bool {
    draft.peer.is_some() && draft.source.root().as_str() == "/"
}

fn peer_safety(draft: &MountDraft) -> Option<&Result<crate::vfs::MountPathCapabilities, String>> {
    draft.peer.as_ref()?.safety.as_ref()
}

fn missing_writes(capabilities: crate::vfs::StagedWriteCapabilities) -> String {
    let mut missing = Vec::new();
    if !capabilities.create {
        missing.push("neue Dateien");
    }
    if !capabilities.replace {
        missing.push("vorhandene Dateien");
    }
    if !capabilities.namespace_replace {
        missing.push("atomare Editor-Ersetzung");
    }
    missing.join(", ")
}

fn mount_root(path: &str) -> Result<crate::mount::BackendRoot, String> {
    crate::mount::BackendRoot::parse(&path.replace('\\', "/")).map_err(|error| error.to_string())
}
