use super::prelude::*;
use super::*;

impl App {
    pub(in crate::app) fn ensure_share(&mut self) -> bool {
        self.share_manual_stop = false;
        if self.share.is_some() {
            return true;
        }
        let server = self.share_server.trim().to_string();
        if server.is_empty() {
            self.share_status = "Kein Share-Server eingetragen".to_string();
            return false;
        }
        self.share_identity
            .set_device_name(self.share_device_draft.clone());
        match crate::share::ShareService::start(
            server,
            self.share_identity.clone(),
            self.share_profiles.clone(),
        ) {
            Ok(svc) => {
                self.share = Some(svc);
                self.share_status = "Share-Server startet".to_string();
                self.configure_share_service();
                true
            }
            Err(e) => {
                self.error_msg = Some(format!("Share-Server-Dienst: {}", e));
                false
            }
        }
    }

    pub(in crate::app) fn share_cmd(&mut self, c: crate::share::ShareCmd) {
        if self.ensure_share() {
            if let Some(svc) = &self.share {
                svc.cmd(c);
            }
        }
    }

    fn configure_share_service(&mut self) {
        if let Some(svc) = &self.share {
            svc.cmd(crate::share::ShareCmd::Configure {
                direct: self.share_profiles.direct_contacts.clone(),
                direct_grants: self.share_profiles.direct_grants.clone(),
                rooms: self.share_profiles.rooms.clone(),
                default_direct_exports: self.share_profiles.default_direct_exports.clone(),
            });
        }
    }

    fn save_share_profiles(&mut self) {
        if let Err(e) = self.share_profiles.save() {
            self.error_msg = Some(format!("Share-Profile speichern: {}", e));
        }
        self.configure_share_service();
    }

    pub(in crate::app) fn drain_quickshare(&mut self) {
        if let Some(qs) = &self.quickshare {
            for list in qs.events.try_iter() {
                self.qs_devices = list;
            }
        }
    }

    pub(in crate::app) fn drain_share(&mut self) {
        if self.share.is_none()
            && self.share_profiles.auto_connect
            && !self.share_manual_stop
            && !self.share_server.trim().is_empty()
        {
            self.ensure_share();
        }

        if let Some(rx) = &self.share_open_rx {
            if let Ok(result) = rx.try_recv() {
                match result {
                    Ok((label, backend, status)) => {
                        self.remote = Some(crate::connect::RemoteState {
                            backend: cache_remote(backend),
                            label: label.clone(),
                            agent_version: None,
                            zip_return: None,
                            sftp: None,
                            account: None,
                            endpoint_prefix: None,
                        });
                        self.net_conn = None;
                        self.notice =
                            Some((format!("Verbunden: {}", label), std::time::Instant::now()));
                        self.start_scan(PathBuf::from("/"));
                        self.mark_opening_status(status);
                    }
                    Err(e) => {
                        self.mark_opening_status(crate::share::ShareStatus::Failed(e.clone()));
                        self.error_msg = Some(format!("Share-Server: {}", e));
                    }
                }
                self.share_open_rx = None;
                self.share_opening = None;
                self.save_share_profiles();
            }
        }

        let events: Vec<crate::share::ShareEvent> = match &self.share {
            Some(svc) => svc.events.try_iter().collect(),
            None => return,
        };
        let mut changed = false;
        let mut auto_open_target: Option<crate::share::PeerOpenTarget> = None;
        for ev in events {
            use crate::share::ShareEvent as E;
            match ev {
                E::Status(s) => {
                    self.share_status = s.clone();
                    self.share_diag_log.push_str(&format!("{s}\n"));
                }
                E::Error(e) => {
                    self.share_status = format!("Fehler: {}", e);
                    self.share_diag_log.push_str(&format!("Fehler: {e}\n"));
                }
                E::ServerConnected => {
                    self.share_status = "Share-Server verbunden".to_string();
                    self.share_diag_log.push_str("Server verbunden\n");
                }
                E::ServerDisconnected(e) => {
                    self.share_status = format!("Share-Server getrennt: {}", e);
                    self.share_diag_log
                        .push_str(&format!("Server getrennt: {e}\n"));
                }
                E::DirectAvailable {
                    lookup_id,
                    presence,
                } => {
                    if let Some(c) = self
                        .share_profiles
                        .direct_contacts
                        .iter_mut()
                        .find(|c| c.lookup_id == lookup_id)
                    {
                        if !c.expected_node_id.trim().is_empty()
                            && c.expected_node_id != presence.node_id
                        {
                            c.status = crate::share::ShareStatus::IdentityConflict;
                            c.last_error = Some("Iroh NodeId passt nicht zum Code".into());
                            changed = true;
                            continue;
                        }
                        if c.expected_node_id.trim().is_empty() {
                            c.expected_node_id = presence.node_id.clone();
                        }
                        c.remote_device_id = Some(presence.device_id.clone());
                        c.remote_public_key = Some(presence.public_key.clone());
                        c.display_name = if c.display_name.trim().is_empty() {
                            presence.device_name.clone()
                        } else {
                            c.display_name.clone()
                        };
                        c.last_seen = Some(crate::share::core_now_secs());
                        c.status = if c.access_state == crate::share::DirectAccessState::Accepted {
                            crate::share::ShareStatus::Available
                        } else {
                            crate::share::ShareStatus::WaitingForAccess
                        };
                        c.last_error = None;
                        c.presence = Some(presence);
                        if c.auto_open
                            && c.access_state == crate::share::DirectAccessState::Accepted
                            && self.share_opening.is_none()
                            && self.remote.is_none()
                        {
                            auto_open_target = Some(crate::share::PeerOpenTarget::Direct {
                                contact_id: c.id.clone(),
                            });
                        }
                        changed = true;
                    }
                }
                E::DirectOffline { lookup_id } => {
                    if let Some(c) = self
                        .share_profiles
                        .direct_contacts
                        .iter_mut()
                        .find(|c| c.lookup_id == lookup_id)
                    {
                        c.status = crate::share::ShareStatus::Offline;
                        c.presence = None;
                        changed = true;
                    }
                }
                E::DirectAccessRequest {
                    lookup_id,
                    presence,
                } => {
                    match self.share_profiles.grant_for(&presence.device_id) {
                        Some(g)
                            if g.public_key == presence.public_key
                                && g.node_id == presence.node_id
                                && g.state == crate::share::DirectGrantState::Accepted =>
                        {
                            self.share_cmd(crate::share::ShareCmd::AnswerDirectRequest {
                                lookup_id,
                                presence,
                                accepted: true,
                            });
                            continue;
                        }
                        Some(g)
                            if g.public_key == presence.public_key
                                && g.node_id == presence.node_id
                                && g.state == crate::share::DirectGrantState::Ignored =>
                        {
                            continue;
                        }
                        Some(_) => {
                            self.share_diag_log.push_str(&format!(
                                "Direct-Anfrage Identitaetskonflikt: {} / {}\n",
                                presence.device_name, presence.device_id
                            ));
                            continue;
                        }
                        None => {}
                    }
                    if !self
                        .share_direct_requests
                        .iter()
                        .any(|p| p.device_id == presence.device_id)
                    {
                        self.share_direct_requests.push(presence.clone());
                    } else if let Some(existing) = self
                        .share_direct_requests
                        .iter_mut()
                        .find(|p| p.device_id == presence.device_id)
                    {
                        *existing = presence.clone();
                    }
                    self.show_share = true;
                    self.share_tab = 0;
                    self.share_status = format!(
                        "Anfrage von {} fuer deinen Direkt-Code",
                        presence.device_name
                    );
                    self.share_diag_log.push_str(&format!(
                        "Direct-Anfrage: lookup={}, device={}, fp={}, candidates={:?}\n",
                        lookup_id, presence.device_name, presence.fingerprint, presence.candidates
                    ));
                }
                E::DirectAccessAccepted {
                    lookup_id,
                    requester_device_id,
                    accepted,
                    presence,
                    msg,
                } => {
                    if requester_device_id != self.share_identity.device_id {
                        continue;
                    }
                    if let Some(c) = self
                        .share_profiles
                        .direct_contacts
                        .iter_mut()
                        .find(|c| c.lookup_id == lookup_id)
                    {
                        if accepted {
                            c.access_state = crate::share::DirectAccessState::Accepted;
                            c.accepted_at = Some(crate::share::core_now_secs());
                            if let Some(p) = presence.clone() {
                                if !c.expected_node_id.trim().is_empty()
                                    && c.expected_node_id != p.node_id
                                {
                                    c.access_state =
                                        crate::share::DirectAccessState::IdentityConflict;
                                    c.status = crate::share::ShareStatus::IdentityConflict;
                                    c.last_error = Some("Iroh NodeId passt nicht zum Code".into());
                                    changed = true;
                                    continue;
                                }
                                if c.expected_node_id.trim().is_empty() {
                                    c.expected_node_id = p.node_id.clone();
                                }
                                c.remote_device_id = Some(p.device_id.clone());
                                c.remote_public_key = Some(p.public_key.clone());
                                c.accepted_public_key = Some(p.public_key.clone());
                                c.presence = Some(p);
                            }
                            c.status = crate::share::ShareStatus::Available;
                            c.last_error = None;
                            changed = true;
                            if c.auto_open && self.share_opening.is_none() && self.remote.is_none()
                            {
                                auto_open_target = Some(crate::share::PeerOpenTarget::Direct {
                                    contact_id: c.id.clone(),
                                });
                            }
                        } else {
                            c.access_state = crate::share::DirectAccessState::Ignored;
                            c.status = crate::share::ShareStatus::Failed(
                                msg.unwrap_or_else(|| "Freigabe abgelehnt".into()),
                            );
                            changed = true;
                        }
                    }
                }
                E::RoomRoster { room_id, members } => {
                    if let Some(r) = self
                        .share_profiles
                        .rooms
                        .iter_mut()
                        .find(|r| r.room_id == room_id)
                    {
                        r.status = crate::share::ShareStatus::Available;
                        r.last_seen = Some(crate::share::core_now_secs());
                        for p in members {
                            if p.device_id != self.share_identity.device_id {
                                upsert_room_member(r, p);
                            }
                        }
                        changed = true;
                    }
                }
                E::RoomJoined { room_id, presence } => {
                    if let Some(r) = self
                        .share_profiles
                        .rooms
                        .iter_mut()
                        .find(|r| r.room_id == room_id)
                    {
                        if presence.device_id != self.share_identity.device_id {
                            upsert_room_member(r, presence);
                            changed = true;
                        }
                    }
                }
                E::RoomLeft { room_id, device_id } => {
                    if let Some(r) = self
                        .share_profiles
                        .rooms
                        .iter_mut()
                        .find(|r| r.room_id == room_id)
                    {
                        if let Some(m) = r.members.iter_mut().find(|m| m.device_id == device_id) {
                            m.status = crate::share::ShareStatus::Offline;
                            m.presence = None;
                            changed = true;
                        }
                    }
                }
            }
        }
        if changed {
            self.save_share_profiles();
        }
        if let Some(target) = auto_open_target {
            self.open_share_target(target);
        }
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
                    self.save_share_profiles();
                    return;
                }
            }
        }
        self.share_opening = Some(target.clone());
        self.mark_opening_status(crate::share::ShareStatus::Connecting);
        let Some(svc) = self.share.clone() else {
            return;
        };
        let (tx, rx) = unbounded();
        std::thread::Builder::new()
            .name("share-open".into())
            .spawn(move || {
                let _ = tx.send(svc.probe_backend_for_target(&target));
            })
            .ok();
        self.share_open_rx = Some(rx);
    }

    fn selected_export_config(&self) -> crate::share::ShareExportConfig {
        match self.share_export_scope {
            1 => self
                .share_profiles
                .direct_contacts
                .iter()
                .find(|c| c.id == self.share_export_target_id)
                .map(|c| c.exports.clone())
                .unwrap_or_else(|| self.share_profiles.default_direct_exports.clone()),
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
        match self.share_export_scope {
            1 => {
                if let Some(c) = self
                    .share_profiles
                    .direct_contacts
                    .iter_mut()
                    .find(|c| c.id == self.share_export_target_id)
                {
                    c.exports = cfg;
                }
            }
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
        self.save_share_profiles();
    }

    pub(in crate::app) fn ui_share(&mut self, ctx: &egui::Context) {
        let mut open = self.show_share;
        let screen = ctx.screen_rect();
        let max_w = (screen.width() - 24.0).max(360.0);
        let max_h = (screen.height() - 24.0).max(360.0);
        egui::Window::new("Share-Server-Verbindungen")
            .open(&mut open)
            .resizable(true)
            .default_size([760.0_f32.min(max_w), 640.0_f32.min(max_h)])
            .max_width(max_w)
            .max_height(max_h)
            .constrain_to(screen.shrink(8.0))
            .show(ctx, |ui| {
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
                match self.share_tab {
                    0 => self.ui_share_direct(ui),
                    1 => self.ui_share_rooms(ui),
                    2 => self.ui_share_exports(ui),
                    _ => self.ui_share_diagnostics(ui),
                }
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
                self.ensure_share();
            }
            if ui.button("Trennen").clicked() {
                if let Some(svc) = &self.share {
                    svc.cmd(crate::share::ShareCmd::Stop);
                }
                self.share = None;
                self.share_manual_stop = true;
                self.share_status = "Getrennt".to_string();
            }
            if ui.button("Aktualisieren").clicked() {
                self.share_cmd(crate::share::ShareCmd::Refresh);
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
        ui.label(
            RichText::new("DIESES GERAET")
                .small()
                .color(Color32::from_gray(140)),
        );
        ui.horizontal_wrapped(|ui| {
            ui.label("Direkt-Code:");
            share_value_field(ui, &self.share_identity.direct_code());
            if ui.button("Code kopieren").clicked() {
                ui.ctx().copy_text(self.share_identity.direct_code());
            }
            if ui.button("Freigaben fuer diesen Code").clicked() {
                self.share_export_scope = 0;
                self.share_export_target_id.clear();
                self.share_tab = 2;
            }
            if ui.button("Fingerprint kopieren").clicked() {
                ui.ctx().copy_text(self.share_identity.fingerprint.clone());
            }
        });
        ui.label(format!(
            "Freigegeben: {}",
            export_summary(&self.share_profiles.default_direct_exports)
        ));
        ui.horizontal_wrapped(|ui| {
            if ui.button("Name aendern").clicked() {
                self.share_identity
                    .set_device_name(self.share_device_draft.clone());
                self.configure_share_service();
            }
            if ui.button("Online schalten").clicked() {
                self.ensure_share();
                self.share_cmd(crate::share::ShareCmd::SetDirectOnline { online: true });
            }
            if ui.button("Offline schalten").clicked() {
                self.share_cmd(crate::share::ShareCmd::SetDirectOnline { online: false });
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
                    self.share_identity.regenerate_direct_code();
                    self.share_regenerate_direct_confirm = false;
                    self.configure_share_service();
                }
                if ui.button("Abbrechen").clicked() {
                    self.share_regenerate_direct_confirm = false;
                }
            });
        }

        if !self.share_direct_requests.is_empty() {
            ui.separator();
            ui.label(
                RichText::new("ANFRAGEN AN DIESES GERAET")
                    .small()
                    .color(Color32::from_gray(140)),
            );
            let mut remove_request: Option<String> = None;
            let requests = self.share_direct_requests.clone();
            for req in requests {
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::Label::new(format!(
                            "{} moechte deinen Direkt-Code nutzen",
                            req.device_name
                        ))
                        .wrap(),
                    );
                    share_value_field(ui, &req.fingerprint);
                    if ui.button("Freigaben waehlen").clicked() {
                        self.share_export_scope = 0;
                        self.share_export_target_id.clear();
                        self.share_tab = 2;
                    }
                    if ui.button("Freigeben").clicked() {
                        self.share_profiles
                            .set_direct_grant(&req, crate::share::DirectGrantState::Accepted);
                        self.save_share_profiles();
                        self.ensure_share();
                        self.share_cmd(crate::share::ShareCmd::AnswerDirectRequest {
                            lookup_id: self.share_identity.direct_lookup_id.clone(),
                            presence: req.clone(),
                            accepted: true,
                        });
                        self.share_cmd(crate::share::ShareCmd::SetDirectOnline { online: true });
                        remove_request = Some(req.device_id.clone());
                        self.notice = Some((
                            format!("Freigabe fuer {} aktiv", req.device_name),
                            std::time::Instant::now(),
                        ));
                    }
                    if ui.button("Ignorieren").clicked() {
                        self.share_profiles
                            .set_direct_grant(&req, crate::share::DirectGrantState::Ignored);
                        self.save_share_profiles();
                        self.share_cmd(crate::share::ShareCmd::AnswerDirectRequest {
                            lookup_id: self.share_identity.direct_lookup_id.clone(),
                            presence: req.clone(),
                            accepted: false,
                        });
                        remove_request = Some(req.device_id.clone());
                    }
                });
            }
            if let Some(device_id) = remove_request {
                self.share_direct_requests
                    .retain(|p| p.device_id != device_id);
            }
        }

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
                    .desired_width(360.0_f32.min(ui.available_width().max(180.0)))
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
                        if let Some(c) = self
                            .share_profiles
                            .direct_contacts
                            .iter_mut()
                            .find(|c| c.id == id)
                        {
                            c.auto_connect = true;
                            c.auto_open = true;
                            c.status = crate::share::ShareStatus::WaitingForAccess;
                        }
                        self.share_direct_code_input.clear();
                        self.share_direct_name_input.clear();
                        self.save_share_profiles();
                        self.ensure_share();
                        self.notice = Some((
                            "Direktgeraet hinzugefuegt, Anfrage gesendet".to_string(),
                            std::time::Instant::now(),
                        ));
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
        let mut changed = false;
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
                if ui.button("Anfrage erneut senden").clicked() {
                    c.access_state = crate::share::DirectAccessState::Pending;
                    c.request_sent_at = Some(crate::share::core_now_secs());
                    c.status = crate::share::ShareStatus::WaitingForAccess;
                    changed = true;
                    request_direct = Some(c.id.clone());
                }
                if ui.checkbox(&mut c.auto_connect, "Auto").changed() {
                    changed = true;
                }
                if ui.checkbox(&mut c.auto_open, "Auto oeffnen").changed() {
                    changed = true;
                }
                if ui.button("Freigaben").clicked() {
                    self.share_export_scope = 1;
                    self.share_export_target_id = c.id.clone();
                    self.share_tab = 2;
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
                    self.share_diag_log.push_str(&format!(
                        "Direct {}: lookup={}, fp={}, status={}, {}\n",
                        c.display_name,
                        c.lookup_id,
                        c.expected_fingerprint,
                        c.status.label(),
                        presence
                    ));
                    self.share_tab = 3;
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
        if let Some(id) = remove {
            self.share_profiles.direct_contacts.retain(|c| c.id != id);
            changed = true;
        }
        if changed {
            self.save_share_profiles();
        }
        if let Some(contact_id) = request_direct {
            self.share_cmd(crate::share::ShareCmd::RequestDirect { contact_id });
        }
        if let Some(target) = open_target {
            self.open_share_target(target);
        }
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
                self.share_room_draft_code = crate::share::ShareProfiles::new_room_code();
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
                        self.share_room_draft_code = crate::share::ShareProfiles::new_room_code();
                        self.save_share_profiles();
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
                    .desired_width(360.0_f32.min(ui.available_width().max(180.0)))
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
                        self.save_share_profiles();
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
        let mut changed = false;
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
                    if let Some(svc) = &self.share {
                        svc.cmd(crate::share::ShareCmd::LeaveRoom {
                            room_id: room.room_id.clone(),
                        });
                    }
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
                    if let Some(code) = crate::share::ShareProfiles::room_code(room) {
                        ui.ctx().copy_text(code);
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
                        self.share_diag_log.push_str(&format!(
                            "Raum {} / {}: fp={}, status={}, {}\n",
                            room.name,
                            member.device_name,
                            member.fingerprint,
                            member.status.label(),
                            presence
                        ));
                        self.share_tab = 3;
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
        if let Some(id) = remove_room {
            self.share_profiles.rooms.retain(|r| r.id != id);
            changed = true;
        }
        if changed {
            self.save_share_profiles();
        }
        if let Some(target) = open_target {
            self.open_share_target(target);
        }
    }

    fn ui_share_exports(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.share_export_scope, 0, "Standard Direkt");
            ui.selectable_value(&mut self.share_export_scope, 1, "Direktgeraet");
            ui.selectable_value(&mut self.share_export_scope, 2, "Raum");
        });
        if self.share_export_scope == 1 {
            egui::ComboBox::from_label("Direktgeraet")
                .selected_text(selected_direct_label(self))
                .show_ui(ui, |ui| {
                    for c in &self.share_profiles.direct_contacts {
                        ui.selectable_value(
                            &mut self.share_export_target_id,
                            c.id.clone(),
                            &c.display_name,
                        );
                    }
                });
        }
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
                    self.share_diag_log.push_str(&format!(
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
                    .desired_width(300.0_f32.min(ui.available_width().max(180.0)))
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
                self.ensure_share();
            }
            if ui.button("Presence neu senden").clicked() {
                self.share_cmd(crate::share::ShareCmd::Refresh);
            }
            if ui.button("Direct Watches neu abonnieren").clicked() {
                self.share_cmd(crate::share::ShareCmd::Refresh);
            }
            if ui.button("Raeume neu beitreten").clicked() {
                self.share_cmd(crate::share::ShareCmd::Refresh);
            }
            if ui.button("Alle Peers pruefen").clicked() {
                self.share_diag_log
                    .push_str("Peer-Pruefung ueber Oeffnen/Diagnose pro Geraet\n");
            }
            if ui.button("Aktiven Peer pruefen").clicked() {
                self.share_diag_log
                    .push_str("Aktiver Peer: Root-Probe laeuft beim Oeffnen\n");
            }
            if ui.button("Log kopieren").clicked() {
                ui.ctx().copy_text(self.share_diag_log.clone());
            }
            if ui.button("Security-Details anzeigen").clicked() {
                let candidates = self
                    .share
                    .as_ref()
                    .map(|svc| svc.peer_candidates())
                    .unwrap_or_default();
                let relay = self
                    .share
                    .as_ref()
                    .map(|svc| svc.relay_url())
                    .unwrap_or_else(|| "-".into());
                self.share_diag_log.push_str(&format!(
                    "device_id={}\nnode_id={}\nfingerprint={}\niroh=aktiv wenn verbunden\nrelay={}\nkandidaten={:?}\n",
                    self.share_identity.device_id,
                    self.share_identity.node_id,
                    self.share_identity.fingerprint,
                    relay,
                    candidates
                ));
            }
        });
        ui.separator();
        ui.label(format!(
            "Listener: {}",
            if self.share.is_some() {
                "aktiv"
            } else {
                "inaktiv"
            }
        ));
        if let Some(svc) = &self.share {
            ui.horizontal_wrapped(|ui| {
                ui.label("Iroh-Relay:");
                share_value_field(ui, &svc.relay_url());
            });
        }
        ui.horizontal_wrapped(|ui| {
            ui.label("Signaling:");
            ui.add(egui::Label::new(self.share_status.clone()).wrap());
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("SmartExplorer-Fingerprint:");
            share_value_field(ui, &self.share_identity.fingerprint);
        });
        egui::ScrollArea::vertical()
            .max_height(420.0)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.share_diag_log.as_str())
                        .font(egui::TextStyle::Monospace)
                        .desired_width(ui.available_width())
                        .desired_rows(18),
                );
            });
    }
}

fn share_value_field(ui: &mut egui::Ui, value: &str) -> egui::Response {
    let mut text = value.to_string();
    let width = ui.available_width().clamp(160.0, 520.0);
    ui.add(
        egui::TextEdit::singleline(&mut text)
            .font(egui::TextStyle::Monospace)
            .desired_width(width)
            .clip_text(true)
            .interactive(false),
    )
}

fn upsert_room_member(room: &mut crate::share::RoomProfile, p: crate::share::PeerPresence) {
    if let Some(m) = room.members.iter_mut().find(|m| m.device_id == p.device_id) {
        if m.public_key != p.public_key || (!m.node_id.is_empty() && m.node_id != p.node_id) {
            m.status = crate::share::ShareStatus::IdentityConflict;
            return;
        }
        m.device_name = p.device_name.clone();
        m.fingerprint = p.fingerprint.clone();
        m.candidates = p.candidates.clone();
        m.node_id = p.node_id.clone();
        m.relay_url = p.relay_url.clone();
        m.last_seen = Some(crate::share::core_now_secs());
        m.status = crate::share::ShareStatus::Available;
        m.presence = Some(p);
    } else {
        room.members.push(crate::share::RoomMember {
            device_id: p.device_id.clone(),
            device_name: p.device_name.clone(),
            fingerprint: p.fingerprint.clone(),
            public_key: p.public_key.clone(),
            node_id: p.node_id.clone(),
            relay_url: p.relay_url.clone(),
            candidates: p.candidates.clone(),
            last_seen: Some(crate::share::core_now_secs()),
            status: crate::share::ShareStatus::Available,
            blocked: false,
            presence: Some(p),
        });
    }
}

fn selected_direct_label(app: &App) -> String {
    app.share_profiles
        .direct_contacts
        .iter()
        .find(|c| c.id == app.share_export_target_id)
        .map(|c| c.display_name.clone())
        .unwrap_or_else(|| "Direktgeraet waehlen".into())
}

fn selected_room_label(app: &App) -> String {
    app.share_profiles
        .rooms
        .iter()
        .find(|r| r.id == app.share_export_target_id)
        .map(|r| r.name.clone())
        .unwrap_or_else(|| "Raum waehlen".into())
}

fn export_summary(cfg: &crate::share::ShareExportConfig) -> String {
    let mut parts = Vec::new();
    if cfg.roots.is_empty() {
        parts.push("keine Ordner".to_string());
    } else if cfg.roots.len() == 1 {
        parts.push(format!("1 Ordner ({})", cfg.roots[0].label));
    } else {
        parts.push(format!("{} Ordner/Laufwerke", cfg.roots.len()));
    }
    if cfg.include_connections {
        parts.push("gespeicherte Verbindungen".to_string());
    }
    parts.join(", ")
}
