//! Durable start intent, retryable stop and explicit installation maintenance.
use super::*;

impl UplinkRuntime {
    pub(super) fn reconcile_at_start(&mut self, now: i64) {
        match UplinkState::load() {
            Ok(state) => {
                self.state = state;
                self.reconciled = true;
            }
            Err(error) => {
                self.state.last_error = Some(format!("Uplink-Status ist nicht lesbar: {error}"));
                self.retry_after = now.saturating_add(5);
                return;
            }
        }
        let Some(record) = self.state.sharing.clone() else {
            return;
        };
        let (private, _) = targets(&record);
        if matches!(self.adapter.sharing_active(&private), Ok(Some(false))) {
            let mut stopped = self.state.clone();
            stopped.sharing = None;
            if let Err(error) = stopped.save() {
                self.state.last_error = Some(error);
                self.stop_due = true;
                self.policy
                    .resume(record.private_index, record.public_index);
            } else {
                self.state = stopped;
            }
        } else {
            // A durable start intent also covers an unknown/partially applied
            // operation. Reconcile it by stopping before any new automatic start.
            self.policy
                .resume(record.private_index, record.public_index);
            self.stop_due = true;
        }
    }

    fn begin_operation(
        &mut self,
        name: &str,
        kind: PendingKind,
        operation: impl FnOnce() -> Result<String, String> + Send + 'static,
    ) {
        let (tx, rx) = channel();
        match std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let _ = tx.send(operation());
            }) {
            Ok(_) => self.pending = Some(Pending { kind, result: rx }),
            Err(error) => {
                self.state.last_error =
                    Some(format!("Uplink-Vorgang konnte nicht starten: {error}"));
                self.retry_after = crate::share::core_now_secs().saturating_add(5);
            }
        }
    }

    pub(super) fn spawn_setup(&mut self) {
        let mut adapter = crate::net::uplink_adapter();
        self.reason = "Einrichtung/Reparatur laeuft (Freigabe bestaetigen)".into();
        self.begin_operation("lan-uplink-setup", PendingKind::Setup, move || {
            adapter.setup_once()
        });
    }

    pub(super) fn spawn_cleanup(&mut self) {
        let mut adapter = crate::net::uplink_adapter();
        self.reason = "Internet-Teilen und privilegierte Installation werden entfernt".into();
        self.begin_operation("lan-uplink-cleanup", PendingKind::Cleanup, move || {
            adapter.cleanup_installation()
        });
    }

    pub(super) fn spawn_start(&mut self, private: UplinkTarget, public: UplinkTarget, now: i64) {
        let record = SharingRecord {
            private_index: private.index,
            private_name: private.name.clone(),
            private_id: private.adapter_id.clone(),
            public_index: public.index,
            public_name: public.name.clone(),
            public_id: public.adapter_id.clone(),
            since: now,
        };
        let mut intent = self.state.clone();
        intent.sharing = Some(record);
        intent.last_error = None;
        if let Err(error) = intent.save() {
            self.state.last_error = Some(format!(
                "Startabsicht konnte nicht gespeichert werden: {error}"
            ));
            self.retry_after = now.saturating_add(5);
            return;
        }
        self.state = intent;
        self.policy.resume(private.index, public.index);
        self.stop_due = true;
        let mut adapter = crate::net::uplink_adapter();
        let (worker_private, worker_public) = (private.clone(), public.clone());
        self.reason = format!(
            "Freigabe wird eingerichtet: {} → {}",
            public.name, private.name
        );
        self.begin_operation(
            "lan-uplink-start",
            PendingKind::Start { private, public },
            move || {
                adapter
                    .enable(&worker_private, &worker_public)
                    .map(|()| "Internet wird geteilt".into())
            },
        );
    }

    pub(super) fn spawn_stop(&mut self, reason: String) {
        let Some(record) = self.state.sharing.clone() else {
            if self.policy.sharing().is_none() {
                self.stop_due = false;
            } else {
                self.state.last_error =
                    Some("Aktive Freigabe ohne dauerhaften Record; Reparatur erforderlich".into());
            }
            return;
        };
        let (private, public) = targets(&record);
        let mut adapter = crate::net::uplink_adapter();
        self.reason = format!("wird beendet: {reason}");
        self.begin_operation("lan-uplink-stop", PendingKind::Stop { reason }, move || {
            adapter
                .disable(&private, &public)
                .map(|()| "Internet-Teilen beendet".into())
        });
    }

    pub(super) fn poll_pending(&mut self, now: i64) {
        let Some(pending) = &self.pending else {
            return;
        };
        let outcome = match pending.result.try_recv() {
            Ok(outcome) => outcome,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("Hintergrundvorgang wurde abgebrochen".into())
            }
        };
        let Some(pending) = self.pending.take() else {
            return;
        };
        match (pending.kind, outcome) {
            (PendingKind::Setup, Ok(message)) => {
                self.setup_message = Some(message);
                self.state.last_error = None;
                if let Err(error) = LanSettings::update(|settings| {
                    settings.uplink_setup_done = true;
                    settings.uplink_repair_requested_at = None;
                    settings.uplink_cleanup_pending = false;
                }) {
                    self.state.last_error = Some(format!(
                        "Einrichtung ist vorhanden, Einstellung speichern: {error}"
                    ));
                }
                self.adapter = crate::net::uplink_adapter();
            }
            (PendingKind::Setup, Err(error)) => {
                self.state.last_error =
                    Some(format!("Einrichtung/Reparatur fehlgeschlagen: {error}"));
                // A failed consent is visible and retryable, never repeated UAC.
                let _ = LanSettings::update(|settings| {
                    settings.uplink_sharing_enabled = false;
                    settings.uplink_repair_requested_at = None;
                });
            }
            (PendingKind::Cleanup, Ok(message)) => {
                self.finish_stopped(message, true);
                self.adapter = crate::net::uplink_adapter();
            }
            (PendingKind::Cleanup, Err(error)) => {
                self.state.last_error = Some(format!(
                    "Entfernung fehlgeschlagen; erneut versuchen: {error}"
                ));
                self.retry_after = now.saturating_add(5);
                // Installation and durable session are retained until both
                // physical disable and removal succeeded. Consent is not looped.
            }
            (PendingKind::Start { private, public }, Ok(_)) => {
                self.policy.mark_started(private.index, public.index);
                self.stop_due = false;
                self.state.last_error = None;
                self.reason = "Internet wird geteilt".into();
            }
            (PendingKind::Start { .. }, Err(error)) => {
                self.state.last_error = Some(format!("Start fehlgeschlagen: {error}"));
                self.stop_due = true;
                self.retry_after = now.saturating_add(5);
            }
            (PendingKind::Stop { reason }, Ok(_)) => self.finish_stopped(reason, false),
            (PendingKind::Stop { .. }, Err(error)) => {
                self.state.last_error = Some(format!(
                    "Beenden fehlgeschlagen; erneuter Versuch folgt: {error}"
                ));
                self.stop_due = true;
                self.retry_after = now.saturating_add(5);
            }
        }
    }

    fn finish_stopped(&mut self, reason: String, cleanup: bool) {
        let mut stopped = self.state.clone();
        stopped.sharing = None;
        stopped.last_error = None;
        if let Err(error) = stopped.save() {
            self.state.last_error = Some(format!(
                "Freigabe beendet, Record loeschen fehlgeschlagen: {error}"
            ));
            self.stop_due = true;
            self.retry_after = crate::share::core_now_secs().saturating_add(5);
            return;
        }
        self.state = stopped;
        self.policy.mark_stopped();
        self.stop_due = false;
        self.reason = reason;
        if let Err(error) = LanSettings::update(|settings| {
            settings.uplink_stop_requested_at = None;
            if cleanup {
                settings.uplink_setup_done = false;
                settings.uplink_cleanup_pending = false;
                settings.uplink_repair_requested_at = None;
            }
        }) {
            self.state.last_error = Some(format!("Uplink-Einstellung speichern: {error}"));
        }
    }

    pub(in crate::daemon) fn shutdown(&mut self) {
        if self.pending.as_ref().is_some_and(|pending| {
            matches!(
                &pending.kind,
                PendingKind::Start { .. } | PendingKind::Stop { .. }
            )
        }) {
            let Some(pending) = self.pending.take() else {
                return;
            };
            if pending
                .result
                .recv_timeout(Duration::from_secs(30))
                .is_err()
            {
                // An in-flight start may still finish. Never clear its intent or
                // claim cancellation; the next daemon reconciles this record.
                log("lan uplink: pending operation at shutdown; durable record retained");
                return;
            }
        }
        let Some(record) = self.state.sharing.clone() else {
            return;
        };
        let (private, public) = targets(&record);
        match self.adapter.disable(&private, &public) {
            Ok(()) => self.finish_stopped("bei Daemon-Ende beendet".into(), false),
            Err(error) => {
                self.state.last_error =
                    Some(format!("Beenden bei Daemon-Ende fehlgeschlagen: {error}"));
                let _ = self.state.save();
            }
        }
    }
}

fn targets(record: &SharingRecord) -> (UplinkTarget, UplinkTarget) {
    (
        UplinkTarget {
            index: record.private_index,
            name: record.private_name.clone(),
            adapter_id: record.private_id.clone(),
        },
        UplinkTarget {
            index: record.public_index,
            name: record.public_name.clone(),
            adapter_id: record.public_id.clone(),
        },
    )
}
