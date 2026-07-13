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
                if let Err(cache_error) = super::profile_cache::reload(self) {
                    self.share_profiles_error = Some(cache_error.clone());
                    self.append_share_diag(format!(
                        "Persistierten Share-Stand ohne Worker laden: {cache_error}"
                    ));
                }
                self.share_next_poll_at = Instant::now() + SHARE_IDLE_POLL;
                return;
            }
        };
        if let Some(status) = super::poll_status::after_successful_snapshot(
            &self.share_status,
            snapshot.running,
            snapshot.connected,
        ) {
            self.share_status = status.to_string();
        }
        // The daemon is the sole owner of runtime Share state. Replacing this
        // cache prevents GUI event replay from writing an older profile over a
        // concurrently persisted request, receipt, decision, or presence.
        self.share_profiles = snapshot.profiles;
        self.share_profiles.storage_revision = snapshot.profile_revision;
        self.share_next_poll_at = Instant::now()
            + if snapshot.running || self.share_profiles.auto_connect {
                SHARE_ACTIVE_POLL
            } else {
                SHARE_IDLE_POLL
            };
        self.share_worker_running = snapshot.running;
        self.share_worker_relay_url = snapshot.relay_url;
        self.share_worker_candidates = snapshot.candidates;
        if self
            .share_profiles
            .legacy_direct_requests
            .iter()
            .any(|entry| entry.is_pending(crate::share::core_now_secs()))
        {
            self.show_share = true;
            self.share_tab = 0;
        }
        let events: Vec<crate::share::ShareEvent> = snapshot.events;
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
                E::DirectSignal(signal) => {
                    use crate::share::DirectSignalEvent as S;
                    match signal {
                        S::RequestReceived {
                            request,
                            received_at,
                        } => {
                            self.show_share = true;
                            self.share_tab = 0;
                            self.share_status = format!(
                                "Getrackte Anfrage {} von {} empfangen",
                                request.request_id, request.requester.device_name
                            );
                            self.append_share_diag(format!(
                                "Tracked request received: id={}, peer={}, at={received_at}",
                                request.request_id, request.requester.device_id
                            ));
                        }
                        S::RequestReceiptReceived {
                            receipt,
                            received_at,
                        } => self.append_share_diag(format!(
                            "Tracked request peer-received: id={}, at={received_at}",
                            receipt.request_id
                        )),
                        S::DecisionReceived {
                            decision,
                            received_at,
                        } => {
                            self.show_share = true;
                            self.share_tab = 0;
                            self.append_share_diag(format!(
                                "Tracked decision received: id={}, decision={}, revision={}, at={received_at}",
                                decision.request_id,
                                decision.decision.code(),
                                decision.decision_revision
                            ));
                        }
                        S::DecisionReceiptReceived {
                            receipt,
                            received_at,
                        } => self.append_share_diag(format!(
                            "Tracked decision peer-received: id={}, revision={}, at={received_at}",
                            receipt.request_id, receipt.decision_revision
                        )),
                        S::EnvelopeAttempted {
                            request_id,
                            envelope,
                            attempt_count,
                            at,
                            failure,
                        } => self.append_share_diag(format!(
                            "Tracked send: id={request_id}, envelope={envelope:?}, attempt={attempt_count}, at={at}, error={failure:?}"
                        )),
                        S::RelayAcknowledged {
                            request_id,
                            envelope,
                            outcome,
                            at,
                        } => self.append_share_diag(format!(
                            "Tracked relay ACK (not peer receipt): id={request_id}, envelope={envelope:?}, outcome={outcome:?}, at={at}"
                        )),
                    }
                }
                E::DirectAvailable {
                    lookup_id,
                    presence,
                } => {
                    if let Some(contact) = self
                        .share_profiles
                        .direct_contacts
                        .iter()
                        .find(|contact| contact.lookup_id == lookup_id)
                    {
                        if contact.auto_open
                            && contact.access_state == crate::share::DirectAccessState::Accepted
                            && can_auto_open
                        {
                            auto_open_target = Some(crate::share::PeerOpenTarget::Direct {
                                contact_id: contact.id.clone(),
                            });
                        }
                    }
                    self.append_share_diag(format!(
                        "Direct online: lookup={lookup_id}, device={}\n",
                        presence.device_name
                    ));
                }
                E::DirectOffline { lookup_id } => {
                    self.append_share_diag(format!("Direct offline: lookup={lookup_id}\n"))
                }
                E::DirectAccessRequest {
                    lookup_id,
                    presence,
                } => {
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
                    presence: _,
                    msg,
                } => {
                    let outcome = if accepted {
                        "accepted".to_string()
                    } else {
                        msg.unwrap_or_else(|| "rejected".to_string())
                    };
                    self.append_share_diag(format!(
                        "Direct decision: lookup={lookup_id}, requester={requester_device_id}, {outcome}\n"
                    ));
                }
                E::RoomRoster { .. } | E::RoomJoined { .. } | E::RoomLeft { .. } => {}
            }
        }
        if let Some(target) = auto_open_target {
            self.open_share_target(target);
        }
    }
}
