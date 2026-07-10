use super::*;

impl App {
    pub(in crate::app) fn drain_share(&mut self) {
        if let Some(svc) = self.share.take() {
            if let Err(error) = svc.cmd(crate::share::ShareCmd::Stop) {
                self.append_share_diag(format!("Lokalen Share-Dienst stoppen: {error}"));
            }
        }

        let mut open_result = None;
        if let Some(rx) = &self.share_open_rx {
            match rx.try_recv() {
                Ok(result) => open_result = Some(result),
                Err(crossbeam_channel::TryRecvError::Empty) => {}
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    open_result = Some(Err("Share-Open-Worker wurde abgebrochen".to_string()));
                }
            }
        }
        if let Some(result) = open_result {
            let opening = self.share_opening.clone();
            let opening_origin = self.share_opening_origin.clone();
            let current_open_origin = self.share_open_context_key();
            let still_current_open = share_open_result_is_current(
                opening.as_ref(),
                self.share_opening.as_ref(),
                opening_origin.as_deref(),
                &current_open_origin,
            );
            match result {
                Ok((label, backend, status)) => {
                    let endpoint_prefix = opening.as_ref().map(|target| target.endpoint_prefix());
                    let already_open = opening
                        .as_ref()
                        .map(|target| self.share_target_is_open(target))
                        .unwrap_or(false);
                    if !still_current_open {
                        self.notice = Some((
                            format!("Share-Verbindung bereit: {}", label),
                            std::time::Instant::now(),
                        ));
                    } else if !already_open {
                        self.remote = Some(crate::connect::RemoteState {
                            backend: cache_remote(backend),
                            label: label.clone(),
                            agent_version: None,
                            zip_return: None,
                            sftp: None,
                            account: None,
                            endpoint_prefix,
                        });
                        self.net_conn = None;
                        self.notice =
                            Some((format!("Verbunden: {}", label), std::time::Instant::now()));
                        self.start_scan(PathBuf::from("/"));
                    } else {
                        self.notice = Some((
                            format!("Bereits verbunden: {}", label),
                            std::time::Instant::now(),
                        ));
                    }
                    self.mark_opening_status(status);
                }
                Err(e) => {
                    self.mark_opening_status(crate::share::ShareStatus::Failed(e.clone()));
                    if still_current_open {
                        self.error_msg = Some(format!("Share-Server: {}", e));
                    } else {
                        self.append_share_diag(format!("Verspaeteter Share-Open Fehler: {e}"));
                    }
                }
            }
            self.share_open_rx = None;
            self.share_opening = None;
            self.share_opening_origin = None;
            let _ = self.persist_share_profiles_only();
        }

        let mut poll_result = None;
        if let Some(rx) = &self.share_poll_rx {
            match rx.try_recv() {
                Ok(result) => {
                    poll_result = Some(result);
                    self.share_poll_rx = None;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    poll_result = Some(Err("Share-Worker Poll abgebrochen".to_string()));
                    self.share_poll_rx = None;
                }
            }
        }
        if self.share_poll_rx.is_none()
            && Instant::now() >= self.share_next_poll_at
            && (self.share_profiles.auto_connect || self.share_worker_running)
        {
            let (tx, rx) = unbounded();
            let spawned = std::thread::Builder::new()
                .name("share-ui-poll".into())
                .spawn(move || {
                    let _ = tx.send(crate::daemon::drain_share_worker_events());
                });
            match spawned {
                Ok(_) => {
                    self.share_poll_rx = Some(rx);
                    self.share_next_poll_at = Instant::now() + SHARE_ACTIVE_POLL;
                }
                Err(e) => {
                    self.share_status = format!("Share-Worker Poll konnte nicht starten: {e}");
                    self.append_share_diag(self.share_status.clone());
                    self.share_next_poll_at = Instant::now() + SHARE_IDLE_POLL;
                }
            }
        }

        let Some(poll_result) = poll_result else {
            return;
        };

        let snapshot = match poll_result {
            Ok(snapshot) => snapshot,
            Err(e) => {
                if self.share_profiles.auto_connect
                    && !self.share_manual_stop
                    && !self.share_server.trim().is_empty()
                {
                    self.share_status = format!("Share-Worker nicht erreichbar: {e}");
                }
                self.share_next_poll_at = Instant::now() + SHARE_IDLE_POLL;
                return;
            }
        };
        self.share_next_poll_at = Instant::now()
            + if snapshot.running || self.share_profiles.auto_connect {
                SHARE_ACTIVE_POLL
            } else {
                SHARE_IDLE_POLL
            };
        self.share_worker_running = snapshot.running;
        self.share_worker_relay_url = snapshot.relay_url;
        self.share_worker_candidates = snapshot.candidates;
        for presence in snapshot.pending_direct_requests {
            if self.share_profiles.grant_for(&presence.device_id).is_none()
                && !self
                    .share_direct_requests
                    .iter()
                    .any(|p| p.device_id == presence.device_id)
            {
                self.share_direct_requests.push(presence);
                self.show_share = true;
                self.share_tab = 0;
            }
        }
        let events: Vec<crate::share::ShareEvent> = snapshot.events;
        let mut changed = false;
        let previous_profiles = self.share_profiles.clone();
        let local_device_id = self
            .share_identity
            .as_ref()
            .map(|identity| identity.device_id.clone());
        let mut auto_open_target: Option<crate::share::PeerOpenTarget> = None;
        let can_auto_open = self.share_can_auto_open();
        for ev in events {
            use crate::share::ShareEvent as E;
            match ev {
                E::Status(s) => {
                    if s.starts_with("Share-Op ") {
                        if self.should_log_share_op() {
                            self.share_status = s.clone();
                            self.append_share_diag(s);
                        }
                    } else {
                        self.share_status = s.clone();
                        self.append_share_diag(s);
                    }
                }
                E::Error(e) => {
                    self.share_status = format!("Fehler: {}", e);
                    self.append_share_diag(format!("Fehler: {e}"));
                }
                E::ServerConnected => {
                    self.share_status = "Share-Server verbunden".to_string();
                    self.append_share_diag("Server verbunden");
                }
                E::ServerDisconnected(e) => {
                    self.share_status = format!("Share-Server getrennt: {}", e);
                    self.append_share_diag(format!("Server getrennt: {e}"));
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
                            && can_auto_open
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
                            let _ = self.share_cmd(crate::share::ShareCmd::AnswerDirectRequest {
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
                            self.append_share_diag(format!(
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
                    self.append_share_diag(format!(
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
                    if local_device_id.as_deref() != Some(requester_device_id.as_str()) {
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
                            if c.auto_open && can_auto_open {
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
                            if local_device_id
                                .as_deref()
                                .is_some_and(|device_id| device_id != p.device_id)
                            {
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
                        if local_device_id
                            .as_deref()
                            .is_some_and(|device_id| device_id != presence.device_id)
                        {
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
            let _ = self.commit_share_profiles(previous_profiles);
        }
        if let Some(target) = auto_open_target {
            self.open_share_target(target);
        }
    }
}
