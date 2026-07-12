use super::prelude::*;
use super::*;

#[path = "share_drain.rs"]
mod drain;
#[path = "share_helpers.rs"]
mod helpers;
#[path = "share_lifecycle_ui.rs"]
mod lifecycle_ui;
#[path = "share_poll_status.rs"]
mod poll_status;
#[path = "share_profile_edits.rs"]
mod profile_edits;
#[path = "share_lifecycle_view.rs"]
mod share_lifecycle_view;

use helpers::*;

const SHARE_ACTIVE_POLL: std::time::Duration = std::time::Duration::from_millis(300);
const SHARE_IDLE_POLL: std::time::Duration = std::time::Duration::from_millis(900);
const SHARE_DIAG_MAX_BYTES: usize = 48 * 1024;

impl App {
    pub(in crate::app) fn ensure_share(&mut self) -> bool {
        self.share_manual_stop = false;
        if let Some(svc) = self.share.take() {
            if let Err(error) = svc.cmd(crate::share::ShareCmd::Stop) {
                self.append_share_diag(format!("Lokalen Share-Dienst stoppen: {error}"));
            }
        }
        let server = self.share_server.trim().to_string();
        if server.is_empty() {
            self.share_status = "Kein Share-Server eingetragen".to_string();
            return false;
        }
        if self.share_identity.is_none() {
            match crate::share::ShareIdentity::load_or_create(default_device_name()) {
                Ok(identity) => {
                    self.share_identity = Some(identity);
                    self.share_identity_error = None;
                }
                Err(error) => {
                    self.share_identity_error = Some(error.clone());
                    self.share_status = format!("Share-Identitaet nicht verfuegbar: {error}");
                    self.error_msg = Some(self.share_status.clone());
                    return false;
                }
            }
        }
        if self.share_profiles_error.is_some() {
            let default_home = dirs_home().to_string_lossy().replace('\\', "/");
            match crate::share::ShareProfiles::load_checked(Some(default_home)) {
                Ok(profiles) => {
                    self.share_profiles = profiles;
                    self.share_profiles_error = None;
                }
                Err(error) => {
                    self.share_profiles_error = Some(error.clone());
                    self.share_status = format!("Share-Profile nicht verfuegbar: {error}");
                    self.error_msg = Some(self.share_status.clone());
                    return false;
                }
            }
        }
        let Some(identity) = self.share_identity.as_mut() else {
            self.share_status = "Share-Identitaet nicht verfuegbar".into();
            self.error_msg = Some(self.share_status.clone());
            return false;
        };
        if let Err(error) = identity.set_device_name(self.share_device_draft.clone()) {
            self.share_status = format!("Share-Geraetename nicht gespeichert: {error}");
            self.error_msg = Some(self.share_status.clone());
            return false;
        }
        if !self.share_profiles.auto_connect {
            let default_home = dirs_home().to_string_lossy().replace('\\', "/");
            match crate::share::ShareProfiles::mutate_persisted(Some(default_home), |profiles| {
                profiles.auto_connect = true;
                Ok(())
            }) {
                Ok(committed) => self.share_profiles = committed,
                Err(error) => {
                    self.share_status = format!("Share-Profile nicht gespeichert: {error}");
                    self.error_msg = Some(self.share_status.clone());
                    return false;
                }
            }
        }
        match crate::daemon::refresh_share_worker_checked() {
            Ok(true) => {
                self.share_status = format!("Share-Worker aktiv ({server})");
                true
            }
            Ok(false) => {
                self.share_status = "Share-Worker wurde nicht aktiv".into();
                self.error_msg = Some(self.share_status.clone());
                false
            }
            Err(error) => {
                self.share_status = format!("Share-Worker konnte nicht aktiviert werden: {error}");
                self.error_msg = Some(self.share_status.clone());
                false
            }
        }
    }

    pub(in crate::app) fn share_cmd(&mut self, c: crate::share::ShareCmd) -> bool {
        if matches!(&c, crate::share::ShareCmd::Stop) {
            let default_home = dirs_home().to_string_lossy().replace('\\', "/");
            match crate::share::ShareProfiles::mutate_persisted(Some(default_home), |profiles| {
                profiles.auto_connect = false;
                Ok(())
            }) {
                Ok(committed) => self.share_profiles = committed,
                Err(error) => {
                    self.share_status = format!("Trennen nicht gespeichert: {error}");
                    self.error_msg = Some(self.share_status.clone());
                    return false;
                }
            }
            if let Err(error) = crate::daemon::send_share_command(c) {
                self.share_status = format!("Share-Worker Stop fehlgeschlagen: {error}");
                self.append_share_diag(self.share_status.clone());
                self.error_msg = Some(self.share_status.clone());
                return false;
            }
            if let Some(svc) = self.share.take() {
                if let Err(error) = svc.cmd(crate::share::ShareCmd::Stop) {
                    self.append_share_diag(format!("Lokalen Share-Dienst stoppen: {error}"));
                }
            }
            self.share_manual_stop = true;
            self.share_worker_running = false;
            self.share_status = "Getrennt".to_string();
            return true;
        }
        if self.ensure_share() {
            if let Err(error) = crate::daemon::send_share_command(c) {
                self.share_status = format!("Share-Worker Fehler: {error}");
                self.append_share_diag(format!("Share-Worker Kommando: {error}"));
                self.error_msg = Some(self.share_status.clone());
                return false;
            }
            return true;
        }
        false
    }

    fn configure_share_service(&mut self) -> bool {
        match crate::daemon::refresh_share_worker_checked() {
            Ok(true) => true,
            Ok(false) => {
                self.error_msg = Some("Share-Worker wurde nicht aktiv".into());
                false
            }
            Err(error) => {
                self.error_msg = Some(format!("Share-Konfiguration zustellen: {error}"));
                false
            }
        }
    }

    fn commit_share_profiles(&mut self, previous: crate::share::ShareProfiles) -> bool {
        let edited = std::mem::replace(&mut self.share_profiles, previous.clone());
        let default_home = dirs_home().to_string_lossy().replace('\\', "/");
        match crate::share::ShareProfiles::mutate_persisted(Some(default_home), |latest| {
            profile_edits::merge_user_edits(latest, &previous, &edited);
            Ok(())
        }) {
            Ok(committed) => {
                self.share_profiles = committed;
                self.configure_share_service()
            }
            Err(error) => {
                self.error_msg = Some(format!("Share-Profile speichern: {error}"));
                false
            }
        }
    }

    fn append_share_diag(&mut self, line: impl AsRef<str>) {
        self.share_diag_log.push_str(line.as_ref());
        if !self.share_diag_log.ends_with('\n') {
            self.share_diag_log.push('\n');
        }
        trim_share_diag_log(&mut self.share_diag_log);
    }

    fn should_log_share_op(&mut self) -> bool {
        if self.share_last_op_log_at.elapsed() < std::time::Duration::from_secs(2) {
            return false;
        }
        self.share_last_op_log_at = Instant::now();
        true
    }

    fn mark_opening_status(&mut self, status: crate::share::ShareStatus) {
        if let Some(target) = &self.share_opening {
            match target {
                crate::share::PeerOpenTarget::Direct { contact_id } => {
                    if let Some(c) = self
                        .share_profiles
                        .direct_contacts
                        .iter_mut()
                        .find(|c| &c.id == contact_id)
                    {
                        c.status = status;
                    }
                }
                crate::share::PeerOpenTarget::RoomDevice { room_id, device_id } => {
                    if let Some(r) = self
                        .share_profiles
                        .rooms
                        .iter_mut()
                        .find(|r| &r.id == room_id || &r.room_id == room_id)
                    {
                        if let Some(m) = r.members.iter_mut().find(|m| &m.device_id == device_id) {
                            m.status = status;
                        }
                    }
                }
            }
        }
    }

    pub(in crate::app) fn open_share_target(&mut self, target: crate::share::PeerOpenTarget) {
        if !self.ensure_share() {
            return;
        }
        if self.share_target_is_open(&target) {
            self.mark_target_status(&target, crate::share::ShareStatus::Connected);
            self.notice = Some((
                "Share-Verbindung ist bereits offen".to_string(),
                std::time::Instant::now(),
            ));
            return;
        }
        if let crate::share::PeerOpenTarget::Direct { contact_id } = &target {
            if let Some(c) = self
                .share_profiles
                .direct_contacts
                .iter_mut()
                .find(|c| &c.id == contact_id)
            {
                if c.access_state != crate::share::DirectAccessState::Accepted {
                    c.status = crate::share::ShareStatus::WaitingForAccess;
                    self.notice = Some((
                        "Warte auf Freigabe am anderen Geraet".to_string(),
                        std::time::Instant::now(),
                    ));
                    return;
                }
            }
        }
        self.share_opening = Some(target.clone());
        self.share_opening_origin = Some(self.share_open_context_key());
        self.mark_target_status(&target, crate::share::ShareStatus::Connecting);
        let (tx, rx) = unbounded();
        let spawned = std::thread::Builder::new()
            .name("share-open".into())
            .spawn(move || {
                let _ = tx.send(crate::daemon::open_share_backend(target));
            });
        match spawned {
            Ok(_) => self.share_open_rx = Some(rx),
            Err(e) => {
                let message = format!("Share-Verbindung konnte nicht gestartet werden: {e}");
                self.mark_opening_status(crate::share::ShareStatus::Failed(message.clone()));
                self.error_msg = Some(message.clone());
                self.append_share_diag(message);
                self.share_opening = None;
                self.share_opening_origin = None;
            }
        }
    }

    fn share_target_is_open(&self, target: &crate::share::PeerOpenTarget) -> bool {
        let Some(remote) = &self.remote else {
            return false;
        };
        remote
            .endpoint_prefix
            .as_deref()
            .map(|prefix| prefix == target.endpoint_prefix())
            .unwrap_or(false)
    }

    fn share_open_context_key(&self) -> String {
        let endpoint = self
            .remote
            .as_ref()
            .and_then(|r| r.endpoint_prefix.clone())
            .unwrap_or_else(|| "local".to_string());
        format!("{endpoint}|{}", self.root_path)
    }

    fn share_can_auto_open(&self) -> bool {
        self.root_path.is_empty()
            && self.remote.is_none()
            && self.net_conn.is_none()
            && self.share_opening.is_none()
            && !self.scan_running
    }

    fn mark_target_status(
        &mut self,
        target: &crate::share::PeerOpenTarget,
        status: crate::share::ShareStatus,
    ) {
        match target {
            crate::share::PeerOpenTarget::Direct { contact_id } => {
                if let Some(c) = self
                    .share_profiles
                    .direct_contacts
                    .iter_mut()
                    .find(|c| &c.id == contact_id)
                {
                    c.status = status;
                }
            }
            crate::share::PeerOpenTarget::RoomDevice { room_id, device_id } => {
                if let Some(r) = self
                    .share_profiles
                    .rooms
                    .iter_mut()
                    .find(|r| &r.id == room_id || &r.room_id == room_id)
                {
                    if let Some(m) = r.members.iter_mut().find(|m| &m.device_id == device_id) {
                        m.status = status;
                    }
                }
            }
        }
    }

    fn selected_export_config(&self) -> crate::share::ShareExportConfig {
        match self.share_export_scope {
            2 => self
                .share_profiles
                .rooms
                .iter()
                .find(|r| r.id == self.share_export_target_id)
                .map(|r| r.exports.clone())
                .unwrap_or_else(|| self.share_profiles.default_direct_exports.clone()),
            _ => self.share_profiles.default_direct_exports.clone(),
        }
    }

    fn set_selected_export_config(&mut self, cfg: crate::share::ShareExportConfig) {
        let previous_profiles = self.share_profiles.clone();
        match self.share_export_scope {
            2 => {
                if let Some(r) = self
                    .share_profiles
                    .rooms
                    .iter_mut()
                    .find(|r| r.id == self.share_export_target_id)
                {
                    r.exports = cfg;
                }
            }
            _ => self.share_profiles.default_direct_exports = cfg,
        }
        let _ = self.commit_share_profiles(previous_profiles);
    }

    fn generate_room_draft_code(&mut self) {
        match crate::share::ShareProfiles::new_room_code() {
            Ok(code) => self.share_room_draft_code = code,
            Err(error) => {
                self.share_room_draft_code.clear();
                self.error_msg = Some(format!("Raum-Code nicht sicher erzeugt: {error}"));
            }
        }
    }

    pub(in crate::app) fn ui_share(&mut self, ctx: &egui::Context) {
        let mut open = self.show_share;
        let screen = ctx.screen_rect();
        let max_w = (screen.width() - 16.0)
            .max(240.0)
            .min(screen.width().max(1.0));
        let max_h = (screen.height() - 16.0)
            .max(240.0)
            .min(screen.height().max(1.0));
        egui::Window::new("Share-Server-Verbindungen")
            .open(&mut open)
            .resizable(true)
            .default_size([760.0_f32.min(max_w), 640.0_f32.min(max_h)])
            .max_width(max_w)
            .max_height(max_h)
            .constrain_to(screen.shrink(8.0))
            .show(ctx, |ui| {
                ui.set_max_width(max_w - 16.0);
                self.ui_share_top(ui);
                ui.separator();
                ui.horizontal(|ui| {
                    for (i, label) in ["Direkt", "Raeume", "Freigaben", "Diagnose"]
                        .iter()
                        .enumerate()
                    {
                        if ui.selectable_label(self.share_tab == i, *label).clicked() {
                            self.share_tab = i;
                        }
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.share_tab {
                        0 => self.ui_share_direct(ui),
                        1 => self.ui_share_rooms(ui),
                        2 => self.ui_share_exports(ui),
                        _ => self.ui_share_diagnostics(ui),
                    });
            });
        self.show_share = open;
    }

    fn ui_share_top(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("share_top_grid")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label("Server:");
                share_value_field(ui, &self.share_server);
                ui.end_row();
                ui.label("Status:");
                ui.add(egui::Label::new(self.share_status.clone()).wrap());
                ui.end_row();
            });
        ui.horizontal_wrapped(|ui| {
            ui.label("Geraet:");
            ui.add(
                egui::TextEdit::singleline(&mut self.share_device_draft)
                    .desired_width(180.0)
                    .clip_text(true),
            );
            if ui.button("Verbinden").clicked() {
                let _ = self.ensure_share();
            }
            if ui.button("Trennen").clicked() {
                let _ = self.share_cmd(crate::share::ShareCmd::Stop);
            }
            if ui.button("Aktualisieren").clicked() {
                let _ = self.share_cmd(crate::share::ShareCmd::Refresh);
            }
            if ui.button("Diagnose").clicked() {
                self.share_tab = 3;
            }
            if ui.button("Server aendern").clicked() {
                self.notice = Some((
                    "Share-Server-Adresse im Einstellungen-Menue aendern".to_string(),
                    std::time::Instant::now(),
                ));
            }
        });
    }

    fn ui_share_direct(&mut self, ui: &mut egui::Ui) {
        let local_identity = self
            .share_identity
            .as_ref()
            .map(|identity| (identity.direct_code(), identity.fingerprint.clone()));
        ui.label(
            RichText::new("DIESES GERAET")
                .small()
                .color(Color32::from_gray(140)),
        );
        ui.horizontal_wrapped(|ui| {
            ui.label("Direkt-Code:");
            if let Some((direct_code, fingerprint)) = &local_identity {
                share_value_field(ui, direct_code);
                if ui.button("Code kopieren").clicked() {
                    ui.ctx().copy_text(direct_code.clone());
                }
                if ui.button("Fingerprint kopieren").clicked() {
                    ui.ctx().copy_text(fingerprint.clone());
                }
            } else {
                ui.colored_label(
                    Color32::from_rgb(255, 120, 120),
                    self.share_identity_error
                        .as_deref()
                        .unwrap_or("Share-Identitaet nicht verfuegbar"),
                );
            }
            if ui.button("Freigaben fuer diesen Code").clicked() {
                self.share_export_scope = 0;
                self.share_export_target_id.clear();
                self.share_tab = 2;
            }
        });
        ui.label(format!(
            "Freigegeben: {}",
            export_summary(&self.share_profiles.default_direct_exports)
        ));
        ui.horizontal_wrapped(|ui| {
            if ui.button("Name aendern").clicked() {
                match self.share_identity.as_mut() {
                    Some(identity) => {
                        match identity.set_device_name(self.share_device_draft.clone()) {
                            Ok(()) => {
                                let _ = self.configure_share_service();
                            }
                            Err(error) => {
                                self.error_msg =
                                    Some(format!("Share-Geraetename nicht gespeichert: {error}"));
                            }
                        }
                    }
                    None => {
                        self.error_msg = Some("Share-Identitaet nicht verfuegbar".into());
                    }
                }
            }
            if ui.button("Online schalten").clicked() {
                let _ = self.ensure_share();
                let _ = self.share_cmd(crate::share::ShareCmd::SetDirectOnline { online: true });
            }
            if ui.button("Offline schalten").clicked() {
                let _ = self.share_cmd(crate::share::ShareCmd::SetDirectOnline { online: false });
            }
            if ui.button("Code neu generieren").clicked() {
                self.share_regenerate_direct_confirm = true;
            }
        });
        if self.share_regenerate_direct_confirm {
            ui.colored_label(
                Color32::from_rgb(255, 185, 120),
                "Neuer Code invalidiert alte Direktkontakte zu diesem Geraet.",
            );
            ui.horizontal_wrapped(|ui| {
                if ui.button("Wirklich neu generieren").clicked() {
                    match self
                        .share_identity
                        .as_mut()
                        .ok_or_else(|| "Share-Identitaet nicht verfuegbar".to_string())
                        .and_then(|identity| identity.regenerate_direct_code())
                    {
                        Ok(outcome) => {
                            self.share_regenerate_direct_confirm = false;
                            let configured = self.configure_share_service();
                            if configured {
                                self.notice = Some((
                                    "Direkt-Code dauerhaft erneuert".into(),
                                    std::time::Instant::now(),
                                ));
                            }
                            if let Some(warning) = outcome.cleanup_warning {
                                self.error_msg = Some(warning);
                            }
                        }
                        Err(error) => {
                            self.error_msg = Some(format!("Direkt-Code nicht erneuert: {error}"));
                        }
                    }
                }
                if ui.button("Abbrechen").clicked() {
                    self.share_regenerate_direct_confirm = false;
                }
            });
        }

        ui.separator();
        lifecycle_ui::ui_lifecycle(self, ui);

        ui.separator();
        ui.label(
            RichText::new("DIREKTGERAET HINZUFUEGEN")
                .small()
                .color(Color32::from_gray(140)),
        );
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.share_direct_code_input)
                    .hint_text("SE-D3-...")
                    .desired_width(share_input_width(ui, 360.0))
                    .clip_text(true),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.share_direct_name_input)
                    .hint_text("Name")
                    .desired_width(140.0),
            );
            if ui.button("Einfuegen").clicked() {
                self.notice = Some((
                    "Bitte mit Strg+V in das Code-Feld einfuegen".to_string(),
                    std::time::Instant::now(),
                ));
            }
            if ui.button("Hinzufuegen").clicked() {
                match self.share_profiles.add_direct_from_code(
                    &self.share_direct_code_input,
                    &self.share_direct_name_input,
                ) {
                    Ok(id) => {
                        self.share_direct_code_input.clear();
                        self.share_direct_name_input.clear();
                        let _ = lifecycle_ui::queue_contact(self, &id);
                    }
                    Err(e) => self.error_msg = Some(e),
                }
            }
            if ui.button("Leeren").clicked() {
                self.share_direct_code_input.clear();
                self.share_direct_name_input.clear();
            }
        });

        ui.separator();
        ui.label(
            RichText::new("GESPEICHERTE DIREKTGERAETE")
                .small()
                .color(Color32::from_gray(140)),
        );
        let mut remove: Option<String> = None;
        let mut open_target: Option<crate::share::PeerOpenTarget> = None;
        let mut request_direct: Option<String> = None;
        let mut pending_diag: Option<String> = None;
        let mut changed = false;
        let previous_profiles = self.share_profiles.clone();
        for c in &mut self.share_profiles.direct_contacts {
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::Label::new(format!(
                        "{} [{} / {}]",
                        c.display_name,
                        c.status.label(),
                        c.access_state.label()
                    ))
                    .wrap(),
                );
                if ui.button("Oeffnen").clicked() {
                    open_target = Some(crate::share::PeerOpenTarget::Direct {
                        contact_id: c.id.clone(),
                    });
                }
                if ui
                    .add_enabled(
                        c.access_state != crate::share::DirectAccessState::Accepted,
                        egui::Button::new("Anfrage senden / erneut versuchen"),
                    )
                    .clicked()
                {
                    request_direct = Some(c.id.clone());
                }
                if ui.checkbox(&mut c.auto_connect, "Auto").changed() {
                    changed = true;
                }
                if ui.checkbox(&mut c.auto_open, "Auto oeffnen").changed() {
                    changed = true;
                }
                if ui.button("Diagnose").clicked() {
                    let presence = c
                        .presence
                        .as_ref()
                        .map(|p| {
                            format!(
                                "node={}, relay={}, candidates={:?}, expires_at={}",
                                p.node_id, p.relay_url, p.candidates, p.expires_at
                            )
                        })
                        .unwrap_or_else(|| "keine Presence".to_string());
                    pending_diag = Some(format!(
                        "Direct {}: lookup={}, fp={}, status={}, {}\n",
                        c.display_name,
                        c.lookup_id,
                        c.expected_fingerprint,
                        c.status.label(),
                        presence
                    ));
                }
                if ui.button("Fingerprint").clicked() {
                    ui.ctx().copy_text(c.expected_fingerprint.clone());
                }
                if ui.button("Trust zuruecksetzen").clicked() {
                    c.remote_device_id = None;
                    c.remote_public_key = None;
                    c.presence = None;
                    c.status = crate::share::ShareStatus::Waiting;
                    changed = true;
                }
                if ui.button("Entfernen").clicked() {
                    remove = Some(c.id.clone());
                }
            });
        }
        if let Some(line) = pending_diag {
            self.append_share_diag(line);
            self.share_tab = 3;
        }
        let persisted = !changed || self.commit_share_profiles(previous_profiles);
        if let Some(id) = remove.filter(|_| persisted) {
            match self.share_profiles.remove_direct_contact(&id) {
                Ok(change) => {
                    if let Some(warning) = change.cleanup_warning {
                        self.error_msg = Some(warning);
                    }
                    if change.changed {
                        let _ = self.configure_share_service();
                    }
                }
                Err(error) => {
                    self.error_msg = Some(format!("Direktgeraet nicht entfernt: {error}"));
                }
            }
        }
        if let Some(contact_id) = request_direct.filter(|_| persisted) {
            let _ = lifecycle_ui::queue_contact(self, &contact_id);
        }
        if let Some(target) = open_target {
            self.open_share_target(target);
        }

        ui.separator();
        egui::CollapsingHeader::new(format!(
            "Quick Share (LAN) — {} gefunden",
            self.qs_devices.len()
        ))
        .id_salt("quickshare_devices")
        .show(ui, |ui| self.ui_quickshare_devices(ui));
    }

    fn ui_share_rooms(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("RAUM ERSTELLEN")
                .small()
                .color(Color32::from_gray(140)),
        );
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.share_room_create_name_input)
                    .desired_width(160.0)
                    .clip_text(true),
            );
            share_value_field(ui, &self.share_room_draft_code);
            if ui.button("Neuen Code").clicked() {
                self.generate_room_draft_code();
            }
            if ui.button("Code kopieren").clicked() {
                ui.ctx().copy_text(self.share_room_draft_code.clone());
            }
            if ui.button("Raum erstellen").clicked() {
                match self.share_profiles.add_room_from_code(
                    &self.share_room_draft_code,
                    &self.share_room_create_name_input,
                ) {
                    Ok(_) => {
                        self.generate_room_draft_code();
                        let _ = self.configure_share_service();
                    }
                    Err(e) => self.error_msg = Some(e),
                }
            }
            if ui.button("Leeren").clicked() {
                self.share_room_create_name_input.clear();
            }
        });

        ui.separator();
        ui.label(
            RichText::new("RAUM BEITRETEN")
                .small()
                .color(Color32::from_gray(140)),
        );
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.share_room_code_input)
                    .hint_text("SE-R3-...")
                    .desired_width(share_input_width(ui, 360.0))
                    .clip_text(true),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.share_room_name_input)
                    .hint_text("Name")
                    .desired_width(120.0),
            );
            if ui.button("Einfuegen").clicked() {
                self.notice = Some((
                    "Bitte mit Strg+V in das Code-Feld einfuegen".to_string(),
                    std::time::Instant::now(),
                ));
            }
            if ui.button("Beitreten").clicked() {
                match self
                    .share_profiles
                    .add_room_from_code(&self.share_room_code_input, &self.share_room_name_input)
                {
                    Ok(_) => {
                        self.share_room_code_input.clear();
                        self.share_room_name_input.clear();
                        let _ = self.configure_share_service();
                    }
                    Err(e) => self.error_msg = Some(e),
                }
            }
            if ui.button("Leeren").clicked() {
                self.share_room_code_input.clear();
                self.share_room_name_input.clear();
            }
        });

        ui.separator();
        ui.label(
            RichText::new("GESPEICHERTE RAEUME")
                .small()
                .color(Color32::from_gray(140)),
        );
        let mut remove_room: Option<String> = None;
        let mut open_target: Option<crate::share::PeerOpenTarget> = None;
        let mut pending_diag: Option<String> = None;
        let mut leave_room: Option<String> = None;
        let mut changed = false;
        let previous_profiles = self.share_profiles.clone();
        for room in &mut self.share_profiles.rooms {
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    egui::Label::new(format!(
                        "{} [{}] Mitglieder: {}",
                        room.name,
                        room.status.label(),
                        room.members.len()
                    ))
                    .wrap(),
                );
                if ui.button("Oeffnen").clicked() {
                    room.status = crate::share::ShareStatus::Available;
                }
                if ui.button("Beitreten").clicked() {
                    room.auto_join = true;
                    changed = true;
                }
                if ui.button("Verlassen").clicked() {
                    leave_room = Some(room.room_id.clone());
                    room.auto_join = false;
                    room.status = crate::share::ShareStatus::Offline;
                    changed = true;
                }
                if ui.checkbox(&mut room.auto_join, "Auto").changed() {
                    changed = true;
                }
                if ui.button("Freigaben").clicked() {
                    self.share_export_scope = 2;
                    self.share_export_target_id = room.id.clone();
                    self.share_tab = 2;
                }
                if ui.button("Code kopieren").clicked() {
                    match crate::share::ShareProfiles::room_code_checked(room) {
                        Ok(Some(code)) => ui.ctx().copy_text(code),
                        Ok(None) => self.error_msg = Some("Raum-Secret fehlt".into()),
                        Err(error) => {
                            self.error_msg = Some(format!("Raum-Code lesen: {error}"));
                        }
                    }
                }
                if ui.button("Umbenennen").clicked() {
                    room.name = self.share_room_name_input.trim().to_string();
                    changed = true;
                }
                if ui.button("Entfernen").clicked() {
                    remove_room = Some(room.id.clone());
                }
            });
            for member in &mut room.members {
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::Label::new(format!(
                            "  {} [{}]",
                            member.device_name,
                            member.status.label()
                        ))
                        .wrap(),
                    );
                    if ui.button("Oeffnen").clicked() {
                        open_target = Some(crate::share::PeerOpenTarget::RoomDevice {
                            room_id: room.id.clone(),
                            device_id: member.device_id.clone(),
                        });
                    }
                    if ui.button("Diagnose").clicked() {
                        let presence = member
                            .presence
                            .as_ref()
                            .map(|p| {
                                format!(
                                    "candidates={:?}, expires_at={}",
                                    p.candidates, p.expires_at
                                )
                            })
                            .unwrap_or_else(|| "keine Presence".to_string());
                        pending_diag = Some(format!(
                            "Raum {} / {}: fp={}, status={}, {}\n",
                            room.name,
                            member.device_name,
                            member.fingerprint,
                            member.status.label(),
                            presence
                        ));
                    }
                    if ui.button("Fingerprint").clicked() {
                        ui.ctx().copy_text(member.fingerprint.clone());
                    }
                    if ui.checkbox(&mut member.blocked, "Blockieren").changed() {
                        changed = true;
                    }
                    if ui.button("Trust zuruecksetzen").clicked() {
                        member.presence = None;
                        member.status = crate::share::ShareStatus::Waiting;
                        changed = true;
                    }
                });
            }
        }
        if let Some(line) = pending_diag {
            self.append_share_diag(line);
            self.share_tab = 3;
        }
        let persisted = !changed || self.commit_share_profiles(previous_profiles);
        if let Some(room_id) = leave_room.filter(|_| persisted) {
            let _ = self.share_cmd(crate::share::ShareCmd::LeaveRoom { room_id });
        }
        if let Some(id) = remove_room.filter(|_| persisted) {
            match self.share_profiles.remove_room(&id) {
                Ok(change) => {
                    if let Some(warning) = change.cleanup_warning {
                        self.error_msg = Some(warning);
                    }
                    if change.changed {
                        let _ = self.configure_share_service();
                    }
                }
                Err(error) => self.error_msg = Some(format!("Raum nicht entfernt: {error}")),
            }
        }
        if let Some(target) = open_target {
            self.open_share_target(target);
        }
    }

    fn ui_share_exports(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.share_export_scope, 0, "Alle Direktkontakte");
            ui.selectable_value(&mut self.share_export_scope, 2, "Raum");
        });
        if self.share_export_scope == 2 {
            egui::ComboBox::from_label("Raum")
                .selected_text(selected_room_label(self))
                .show_ui(ui, |ui| {
                    for r in &self.share_profiles.rooms {
                        ui.selectable_value(
                            &mut self.share_export_target_id,
                            r.id.clone(),
                            &r.name,
                        );
                    }
                });
        }

        let mut cfg = self.selected_export_config();
        let mut remove: Option<usize> = None;
        let mut move_up: Option<usize> = None;
        let mut move_down: Option<usize> = None;
        let mut changed = false;
        if ui
            .checkbox(
                &mut cfg.include_connections,
                "Eigene gespeicherte Verbindungen freigeben",
            )
            .changed()
        {
            changed = true;
        }
        if ui
            .checkbox(
                &mut cfg.allow_exec,
                "Vollstaendige Remote-Codeausfuehrung erlauben",
            )
            .changed()
        {
            changed = true;
        }
        if cfg.allow_exec {
            ui.small(
                "Umfasst Programme und Shell-Befehle; die Ausfuehrung ist derzeit bis zur sicheren Prozessbaum-Kapselung deaktiviert.",
            );
        }
        ui.checkbox(
            &mut self.share_block_symlink_escape,
            "Symlinks ausserhalb der Freigabe blockieren",
        );
        ui.add_enabled(
            false,
            egui::Checkbox::new(&mut true, "Share-Server-Verbindungen ausschliessen"),
        );
        for (i, root) in cfg.roots.iter().enumerate() {
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::Label::new(format!("{} ->", root.label)).wrap());
                share_value_field(ui, &root.path);
                if ui.button("Test").clicked() {
                    self.append_share_diag(format!(
                        "Freigabe-Test {}: {}\n",
                        root.label,
                        if std::path::Path::new(&root.path).exists() {
                            "ok"
                        } else {
                            "nicht gefunden"
                        }
                    ));
                }
                if ui.button("Nach oben").clicked() && i > 0 {
                    move_up = Some(i);
                }
                if ui.button("Nach unten").clicked() && i + 1 < cfg.roots.len() {
                    move_down = Some(i);
                }
                if ui.button("Entfernen").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = move_up {
            cfg.roots.swap(i, i - 1);
            changed = true;
        }
        if let Some(i) = move_down {
            cfg.roots.swap(i, i + 1);
            changed = true;
        }
        if let Some(i) = remove {
            cfg.roots.remove(i);
            changed = true;
        }
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.share_export_label_draft)
                    .hint_text("Name")
                    .desired_width(120.0),
            );
            ui.add(
                egui::TextEdit::singleline(&mut self.share_export_path_draft)
                    .hint_text("Ordner, Laufwerk oder UNC")
                    .desired_width(share_input_width(ui, 300.0))
                    .clip_text(true),
            );
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Ordner hinzufuegen").clicked() {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    self.share_export_path_draft = p.to_string_lossy().replace('\\', "/");
                }
            }
            if ui.button("Aktuellen Ordner hinzufuegen").clicked()
                && self.remote.is_none()
                && !self.root_path.is_empty()
            {
                self.share_export_path_draft = self.root_path.clone();
            }
            if ui.button("Laufwerk hinzufuegen").clicked() {
                if let Some(d) = self.drives.first() {
                    self.share_export_path_draft = d.clone();
                }
            }
            if ui.button("Alle Laufwerke hinzufuegen").clicked() {
                for d in self.drives.clone() {
                    let label = d.trim_end_matches(['\\', '/']).to_string();
                    if !cfg.roots.iter().any(|r| r.path == d) {
                        cfg.roots.push(crate::share::SharedRoot { label, path: d });
                        changed = true;
                    }
                }
            }
            if ui.button("Gespeicherte Verbindung hinzufuegen").clicked() {
                cfg.include_connections = true;
                changed = true;
            }
            if ui.button("Alle gespeicherten Verbindungen").clicked() {
                cfg.include_connections = true;
                changed = true;
            }
            if ui.button("Hinzufuegen").clicked() {
                let path = self.share_export_path_draft.trim().replace('\\', "/");
                if !path.is_empty() && !cfg.roots.iter().any(|r| r.path == path) {
                    cfg.roots.push(crate::share::SharedRoot {
                        label: self.share_export_label_draft.trim().to_string(),
                        path,
                    });
                    changed = true;
                }
            }
            if ui.button("Alles entfernen").clicked() {
                cfg.roots.clear();
                changed = true;
            }
        });
        if changed {
            self.set_selected_export_config(cfg);
        }
    }

    fn ui_share_diagnostics(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Server testen").clicked() {
                let _ = self.ensure_share();
            }
            if ui.button("Presence neu senden").clicked() {
                let _ = self.share_cmd(crate::share::ShareCmd::Refresh);
            }
            if ui.button("Direct Watches neu abonnieren").clicked() {
                let _ = self.share_cmd(crate::share::ShareCmd::Refresh);
            }
            if ui.button("Raeume neu beitreten").clicked() {
                let _ = self.share_cmd(crate::share::ShareCmd::Refresh);
            }
            if ui.button("Alle Peers pruefen").clicked() {
                self.append_share_diag("Peer-Pruefung ueber Oeffnen/Diagnose pro Geraet");
            }
            if ui.button("Aktiven Peer pruefen").clicked() {
                self.append_share_diag("Aktiver Peer: Root-Probe laeuft beim Oeffnen");
            }
            if ui.button("Log kopieren").clicked() {
                ui.ctx().copy_text(self.share_diag_log.clone());
            }
            if ui.button("Security-Details anzeigen").clicked() {
                if let Some(identity) = &self.share_identity {
                    self.append_share_diag(format!(
                        "device_id={}\nnode_id={}\nfingerprint={}\niroh=aktiv wenn verbunden\nrelay={}\nkandidaten={:?}\n",
                        identity.device_id,
                        identity.node_id,
                        identity.fingerprint,
                        self.share_worker_relay_url,
                        self.share_worker_candidates
                    ));
                } else {
                    self.append_share_diag(
                        self.share_identity_error
                            .clone()
                            .unwrap_or_else(|| "Share-Identitaet nicht verfuegbar".into()),
                    );
                }
            }
        });
        ui.separator();
        ui.label(format!(
            "Listener: {}",
            if self.share_worker_running {
                "aktiv"
            } else {
                "inaktiv"
            }
        ));
        if !self.share_worker_relay_url.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label("Iroh-Relay:");
                share_value_field(ui, &self.share_worker_relay_url);
            });
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Signaling:");
            ui.add(egui::Label::new(self.share_status.clone()).wrap());
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("SmartExplorer-Fingerprint:");
            let fingerprint = self
                .share_identity
                .as_ref()
                .map(|identity| identity.fingerprint.as_str())
                .unwrap_or("nicht verfuegbar");
            share_value_field(ui, fingerprint);
        });
        egui::ScrollArea::vertical()
            .max_height(420.0)
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(self.share_diag_log.as_str())
                            .monospace()
                            .color(Color32::from_gray(210)),
                    )
                    .wrap(),
                );
            });
    }
}
