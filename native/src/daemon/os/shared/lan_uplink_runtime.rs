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

#[path = "lan_uplink_operations.rs"]
mod operations;

const INTERNET_PROBE_INTERVAL: Duration = Duration::from_secs(20);

enum PendingKind {
    Setup,
    Cleanup,
    Start {
        private: UplinkTarget,
        public: UplinkTarget,
    },
    Stop {
        reason: String,
    },
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
    setup_attempted: bool,
    cleanup_attempted: bool,
    repair_attempt: Option<i64>,
    retry_after: i64,
    stop_due: bool,
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
            setup_attempted: false,
            cleanup_attempted: false,
            repair_attempt: None,
            retry_after: 0,
            stop_due: false,
        }
    }

    /// Interfaces this host currently shares on (for link classification).
    pub(super) fn shared_ifaces(&self) -> Vec<u32> {
        self.policy
            .sharing()
            .map(|(private_if, _)| vec![private_if])
            .unwrap_or_default()
    }

    /// Only the fresh cached platform verdict; never probes on the caller's thread.
    pub(super) fn internet_verdict_snapshot(&self) -> Option<&[u32]> {
        self.internet_probed_at
            .filter(|at| at.elapsed() <= INTERNET_PROBE_INTERVAL)
            .and(self.internet_ifaces.as_deref())
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
        if !self.reconciled && input.now >= self.retry_after {
            self.reconcile_at_start(input.now);
        }
        let explicit_repair = input.settings.uplink_repair_requested_at.is_some()
            && input.settings.uplink_repair_requested_at != self.repair_attempt;
        if self.pending.is_none() && self.reconciled {
            if !input.settings.uplink_sharing_enabled && input.settings.uplink_cleanup_pending
                && (!self.cleanup_attempted || explicit_repair) {
                self.cleanup_attempted = true;
                self.repair_attempt = input.settings.uplink_repair_requested_at;
                self.spawn_cleanup();
            } else if input.settings.uplink_sharing_enabled && (explicit_repair
                || (!input.settings.uplink_setup_done && !self.setup_attempted
                    && !matches!(&self.facility, Facility::RepairRequired(_)))) {
                self.setup_attempted = true;
                self.repair_attempt = input.settings.uplink_repair_requested_at;
                self.spawn_setup();
            }
        }
        if !input.settings.uplink_cleanup_pending { self.cleanup_attempted = false; }
        if !input.settings.uplink_sharing_enabled { self.setup_attempted = false; }
        let internet = self.internet_ifaces(input.facts);
        let shared = self.shared_ifaces();
        let links = crate::net::classify_links(
            input.facts,
            input.peer_ifaces,
            internet.as_deref(),
            &shared,
        );
        let stop_requested = input.settings.uplink_stop_requested_at.is_some();
        let decision = if self.pending.is_some() || !self.reconciled || input.now < self.retry_after {
            Decision::Keep
        } else if self.stop_due {
            Decision::Stop("Start/Stop wird sicher abgeglichen".into())
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
                    self.spawn_start(private, public, input.now);
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
                PendingKind::Cleanup => UplinkSharingState::Stopping,
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
            Facility::Unavailable(reason) | Facility::RepairRequired(reason) => LanFacility::Unavailable(reason.clone()),
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
            repair_required: matches!(&self.facility, Facility::RepairRequired(_)) || self.state.last_error.is_some() || settings.uplink_cleanup_pending,
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
