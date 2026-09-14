//! Daemon-side driver of automatic uplink sharing: probes the platform
//! adapter, runs the one-time setup when the user opts in, evaluates the
//! pure policy every tick, applies start/stop decisions off the tick thread,
//! persists the active session for restart reconciliation, and renders the
//! `UplinkView`.
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

use crate::net::{
    Facility, InterfaceFacts, LinkClass, SharingRecord, UplinkAdapter, UplinkState, UplinkTarget,
};
use crate::share::lan_uplink_policy::{Decision, PeerOnLink, PolicyInput, UplinkPolicy};
use crate::share::{LanFacility, LanSettings, UplinkSharingState, UplinkView};

use super::state::log;

const INTERNET_PROBE_INTERVAL: Duration = Duration::from_secs(20);

enum PendingKind {
    Setup,
    Start { private: UplinkTarget, public: UplinkTarget },
    Stop { reason: String },
}

struct Pending {
    kind: PendingKind,
    result: Receiver<Result<String, String>>,
}

pub(super) struct UplinkRuntime {
    adapter: Box<dyn UplinkAdapter>,
    policy: UplinkPolicy,
    state: UplinkState,
    pending: Option<Pending>,
    reconciled: bool,
    facility: Facility,
    internet_ifaces: Option<Vec<u32>>,
    internet_probed_at: Option<Instant>,
    reason: String,
    setup_message: Option<String>,
}

pub(super) struct UplinkTickInput<'a> {
    pub(super) settings: &'a LanSettings,
    pub(super) facts: &'a [InterfaceFacts],
    pub(super) peer_ifaces: &'a [u32],
    pub(super) peers: &'a [PeerOnLink],
    pub(super) own_hashed_id: &'a str,
    pub(super) now: i64,
}

impl UplinkRuntime {
    pub(super) fn new() -> Self {
        Self {
            adapter: crate::net::uplink_adapter(),
            policy: UplinkPolicy::default(),
            state: UplinkState::default(),
            pending: None,
            reconciled: false,
            facility: Facility::Unavailable("noch nicht geprueft".into()),
            internet_ifaces: None,
            internet_probed_at: None,
            reason: String::new(),
            setup_message: None,
        }
    }

    /// Interfaces this host currently shares on (for link classification).
    pub(super) fn shared_ifaces(&self) -> Vec<u32> {
        self.policy
            .sharing()
            .map(|(private_if, _)| vec![private_if])
            .unwrap_or_default()
    }

    /// Let the platform refine DHCP knowledge before classification.
    pub(super) fn refine_facts(&mut self, facts: &mut [InterfaceFacts]) {
        self.adapter.refine_facts(facts);
    }

    /// Platform internet verdict, refreshed at most every 20 s.
    pub(super) fn internet_ifaces(&mut self, facts: &[InterfaceFacts]) -> Option<Vec<u32>> {
        let due = self
            .internet_probed_at
            .is_none_or(|at| at.elapsed() >= INTERNET_PROBE_INTERVAL);
        if due {
            self.internet_ifaces = self.adapter.internet_ifaces(facts);
            self.internet_probed_at = Some(Instant::now());
        }
        self.internet_ifaces.clone()
    }

    pub(super) fn tick(&mut self, input: UplinkTickInput<'_>) -> UplinkView {
        self.poll_pending(input.now);
        self.facility = self.adapter.probe(input.settings.uplink_setup_done);
        if !self.reconciled {
            self.reconcile_at_start();
        }
        if input.settings.uplink_sharing_enabled
            && !input.settings.uplink_setup_done
            && self.pending.is_none()
        {
            self.spawn_setup();
        }
        let internet = self.internet_ifaces(input.facts);
        let shared = self.shared_ifaces();
        let links = crate::net::classify_links(
            input.facts,
            input.peer_ifaces,
            internet.as_deref(),
            &shared,
        );
        let stop_requested = input.settings.uplink_stop_requested_at.is_some();
        let decision = if self.pending.is_some() {
            Decision::Keep
        } else {
            self.policy.evaluate(
                &PolicyInput {
                    enabled: input.settings.uplink_sharing_enabled,
                    adapter_available: self.facility.is_available(),
                    links: &links,
                    peers: input.peers,
                    own_hashed_id: input.own_hashed_id,
                    stop_requested,
                },
                input.now,
            )
        };
        match decision {
            Decision::Keep => {}
            Decision::Idle(reason) => self.reason = reason,
            Decision::Start {
                private_if,
                public_if,
            } => {
                let private = links
                    .iter()
                    .find(|(facts, _)| facts.index == private_if)
                    .map(|(facts, _)| UplinkTarget::from_facts(facts));
                let public = links
                    .iter()
                    .find(|(facts, _)| facts.index == public_if)
                    .map(|(facts, _)| UplinkTarget::from_facts(facts));
                if let (Some(private), Some(public)) = (private, public) {
                    self.spawn_start(private, public);
                }
            }
            Decision::Stop(reason) => self.spawn_stop(reason),
        }
        if stop_requested && self.policy.sharing().is_none() && self.pending.is_none() {
            // Nothing to stop (or already stopped): consume the request.
            let _ = LanSettings::update(|settings| settings.uplink_stop_requested_at = None);
        }
        self.view(input.settings, &links)
    }

    fn reconcile_at_start(&mut self) {
        self.reconciled = true;
        match UplinkState::load() {
            Ok(state) => self.state = state,
            Err(error) => {
                log(&format!("lan uplink: state unreadable: {error}"));
                return;
            }
        }
        let Some(record) = self.state.sharing.clone() else {
            return;
        };
        let private = UplinkTarget {
            index: record.private_index,
            name: record.private_name.clone(),
            adapter_id: record.private_id.clone(),
        };
        match self.adapter.sharing_active(&private) {
            Ok(Some(true)) | Ok(None) => {
                log("lan uplink: resuming sharing session found at start");
                self.policy.resume(record.private_index, record.public_index);
            }
            Ok(Some(false)) => {
                log("lan uplink: recorded session is no longer active; clearing");
                self.state.sharing = None;
                let _ = self.state.save();
            }
            Err(error) => {
                log(&format!("lan uplink: could not verify recorded session: {error}"));
                self.policy.resume(record.private_index, record.public_index);
            }
        }
    }

    fn spawn_setup(&mut self) {
        let (tx, rx) = channel();
        let mut adapter = crate::net::uplink_adapter();
        let spawned = std::thread::Builder::new()
            .name("lan-uplink-setup".into())
            .spawn(move || {
                let _ = tx.send(adapter.setup_once());
            });
        match spawned {
            Ok(_) => {
                self.pending = Some(Pending {
                    kind: PendingKind::Setup,
                    result: rx,
                });
                self.reason = "einmalige Einrichtung laeuft (Freigabe bestaetigen)".into();
            }
            Err(error) => {
                self.state.last_error = Some(format!("Einrichtung konnte nicht starten: {error}"));
            }
        }
    }

    fn spawn_start(&mut self, private: UplinkTarget, public: UplinkTarget) {
        let (tx, rx) = channel();
        let mut adapter = crate::net::uplink_adapter();
        let (private_thread, public_thread) = (private.clone(), public.clone());
        let spawned = std::thread::Builder::new()
            .name("lan-uplink-start".into())
            .spawn(move || {
                let _ = tx.send(
                    adapter
                        .enable(&private_thread, &public_thread)
                        .map(|()| "Internet wird geteilt".to_string()),
                );
            });
        match spawned {
            Ok(_) => {
                log(&format!(
                    "lan uplink: starting sharing {} -> {}",
                    public.name, private.name
                ));
                self.reason = format!("Freigabe wird eingerichtet: {} → {}", public.name, private.name);
                self.pending = Some(Pending {
                    kind: PendingKind::Start { private, public },
                    result: rx,
                });
            }
            Err(error) => {
                self.state.last_error = Some(format!("Freigabe konnte nicht starten: {error}"));
            }
        }
    }

    fn spawn_stop(&mut self, reason: String) {
        let Some(record) = self.state.sharing.clone() else {
            // Sharing known to the policy only (resumed without record).
            if let Some((private_if, public_if)) = self.policy.sharing() {
                log(&format!(
                    "lan uplink: stopping session without record ({private_if}->{public_if}): {reason}"
                ));
            }
            self.policy.mark_stopped();
            self.reason = reason;
            return;
        };
        let private = UplinkTarget {
            index: record.private_index,
            name: record.private_name.clone(),
            adapter_id: record.private_id.clone(),
        };
        let public = UplinkTarget {
            index: record.public_index,
            name: record.public_name.clone(),
            adapter_id: record.public_id.clone(),
        };
        let (tx, rx) = channel();
        let mut adapter = crate::net::uplink_adapter();
        let spawned = std::thread::Builder::new()
            .name("lan-uplink-stop".into())
            .spawn(move || {
                let _ = tx.send(
                    adapter
                        .disable(&private, &public)
                        .map(|()| "Internet-Teilen beendet".to_string()),
                );
            });
        match spawned {
            Ok(_) => {
                log(&format!("lan uplink: stopping sharing: {reason}"));
                self.reason = format!("wird beendet: {reason}");
                self.pending = Some(Pending {
                    kind: PendingKind::Stop { reason },
                    result: rx,
                });
            }
            Err(error) => {
                self.state.last_error = Some(format!("Beenden konnte nicht starten: {error}"));
            }
        }
    }

    fn poll_pending(&mut self, now: i64) {
        let Some(pending) = &self.pending else {
            return;
        };
        let outcome = match pending.result.try_recv() {
            Ok(outcome) => outcome,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("Hintergrundvorgang wurde abgebrochen".to_string())
            }
        };
        let Some(pending) = self.pending.take() else {
            return;
        };
        match (pending.kind, outcome) {
            (PendingKind::Setup, Ok(message)) => {
                log(&format!("lan uplink: setup done: {message}"));
                self.setup_message = Some(message);
                self.state.last_error = None;
                let _ = LanSettings::update(|settings| settings.uplink_setup_done = true);
            }
            (PendingKind::Setup, Err(error)) => {
                log(&format!("lan uplink: setup failed: {error}"));
                self.state.last_error = Some(format!("Einrichtung fehlgeschlagen: {error}"));
                // Do not retry in a loop: the user re-enables the setting to try again.
                let _ = LanSettings::update(|settings| settings.uplink_sharing_enabled = false);
            }
            (PendingKind::Start { private, public }, Ok(_)) => {
                log(&format!(
                    "lan uplink: sharing active {} -> {}",
                    public.name, private.name
                ));
                self.policy.mark_started(private.index, public.index);
                self.state.sharing = Some(SharingRecord {
                    private_index: private.index,
                    private_name: private.name,
                    private_id: private.adapter_id,
                    public_index: public.index,
                    public_name: public.name,
                    public_id: public.adapter_id,
                    since: now,
                });
                self.state.last_error = None;
                self.reason = "Internet wird geteilt".into();
                if let Err(error) = self.state.save() {
                    log(&format!("lan uplink: state not saved: {error}"));
                }
            }
            (PendingKind::Start { .. }, Err(error)) => {
                log(&format!("lan uplink: start failed: {error}"));
                self.state.last_error = Some(error);
                self.policy.mark_stopped();
            }
            (PendingKind::Stop { reason }, Ok(_)) => {
                log(&format!("lan uplink: sharing stopped: {reason}"));
                self.policy.mark_stopped();
                self.state.sharing = None;
                self.state.last_error = None;
                self.reason = reason;
                let _ = self.state.save();
                let _ = LanSettings::update(|settings| settings.uplink_stop_requested_at = None);
            }
            (PendingKind::Stop { reason }, Err(error)) => {
                log(&format!("lan uplink: stop failed: {error}"));
                self.state.last_error = Some(format!("Beenden fehlgeschlagen: {error}"));
                self.policy.mark_stopped();
                self.state.sharing = None;
                self.reason = reason;
                let _ = self.state.save();
                let _ = LanSettings::update(|settings| settings.uplink_stop_requested_at = None);
            }
        }
    }

    /// Stop an active session synchronously (daemon shutdown).
    pub(super) fn shutdown(&mut self) {
        let Some(record) = self.state.sharing.take() else {
            return;
        };
        let private = UplinkTarget {
            index: record.private_index,
            name: record.private_name,
            adapter_id: record.private_id,
        };
        let public = UplinkTarget {
            index: record.public_index,
            name: record.public_name,
            adapter_id: record.public_id,
        };
        match self.adapter.disable(&private, &public) {
            Ok(()) => log("lan uplink: sharing stopped at shutdown"),
            Err(error) => log(&format!("lan uplink: stop at shutdown failed: {error}")),
        }
        self.policy.mark_stopped();
        let _ = self.state.save();
    }

    fn view(&self, settings: &LanSettings, links: &[(InterfaceFacts, LinkClass)]) -> UplinkView {
        let name_of = |index: u32| {
            links
                .iter()
                .find(|(facts, _)| facts.index == index)
                .map(|(facts, _)| facts.name.clone())
                .unwrap_or_else(|| format!("#{index}"))
        };
        let state = match (&self.pending, self.policy.sharing()) {
            (Some(pending), _) => match pending.kind {
                PendingKind::Setup => UplinkSharingState::Idle,
                PendingKind::Start { .. } => UplinkSharingState::Starting,
                PendingKind::Stop { .. } => UplinkSharingState::Stopping,
            },
            (None, Some(_)) => UplinkSharingState::Sharing,
            (None, None) => UplinkSharingState::Idle,
        };
        let (private_if, public_if) = match self.policy.sharing() {
            Some((private_if, public_if)) => (Some(name_of(private_if)), Some(name_of(public_if))),
            None => (None, None),
        };
        let facility = match &self.facility {
            Facility::Available => LanFacility::Available,
            Facility::Unavailable(reason) => LanFacility::Unavailable(reason.clone()),
        };
        let mut reason = self.reason.clone();
        if let Some(message) = &self.setup_message {
            if reason.is_empty() {
                reason = message.clone();
            }
        }
        UplinkView {
            enabled: settings.uplink_sharing_enabled,
            setup_done: settings.uplink_setup_done,
            facility,
            state,
            reason,
            private_if,
            public_if,
            since: self.state.sharing.as_ref().map(|record| record.since),
            last_error: self.state.last_error.clone(),
        }
    }
}
