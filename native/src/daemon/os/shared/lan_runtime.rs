//! Daemon-side owner of local-network presence: starts the mDNS announcer
//! when enabled, keeps the announcement in sync with the Iroh ports and the
//! uplink verdict, turns sightings of paired peers into `ShareEvent`s, expires
//! stale evidence, and renders the `LanStatus` snapshot for GUI and CLI.
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::net::{InterfaceFacts, LinkClass};
use crate::share::lan_link_facts::{
    self, AuthenticatedLanFact, LanLinkHostFacts, LanPeerPin, OwnUplink,
};
use crate::share::lan_uplink_policy::PeerOnLink;
use crate::share::{
    lan_presence_match, DirectContact, DirectGrant, LanEvent, LanFacility, LanPeerView,
    LanPresence, LanSettings, LanSighting, LanStatus, LinkView, ShareEvent, UplinkView,
    LAN_PRESENCE_TTL_SECS,
};

use super::lan_uplink_runtime::{UplinkRuntime, UplinkTickInput};

const FACTS_INTERVAL: Duration = Duration::from_secs(5);
const START_RETRY: Duration = Duration::from_secs(60);

#[path = "lan_runtime_presence.rs"]
mod presence_operations;

pub(super) struct LanTickInput<'a> {
    pub(super) contacts: &'a [DirectContact],
    pub(super) grants: &'a [DirectGrant],
    pub(super) paired_links: &'a [AuthenticatedLanFact],
    pub(super) own_node_id: Option<&'a str>,
    pub(super) ports: (Option<u16>, Option<u16>),
    pub(super) uplink_advisory: bool,
    pub(super) now: i64,
}

pub(super) struct LanRuntime {
    settings: LanSettings,
    settings_error: Option<String>,
    presence: Option<LanPresence>,
    presence_error: Option<String>,
    last_start_attempt: Option<Instant>,
    facts: Vec<InterfaceFacts>,
    facts_error: Option<String>,
    last_facts_at: Option<Instant>,
    sightings: HashMap<String, LanSighting>,
    proofs: HashMap<String, crate::share::LanProof>,
    authenticator: crate::share::lan_presence_auth::LanAuthenticator,
    authenticated_contacts: Vec<String>,
    link_facts: Vec<AuthenticatedLanFact>,
    uplink_pins: Vec<LanPeerPin>,
    evidence_revision: Option<Vec<AuthenticatedLanFact>>,
    /// contact id → (candidates, uplink) currently reported as seen.
    reported: HashMap<String, (Vec<String>, bool)>,
    /// Hashed ids of the sightings that matched a paired contact.
    reported_hashes: Vec<String>,
    unknown_devices: usize,
    /// Interface indexes this host currently shares its uplink on (Stage 2).
    shared_ifaces: Vec<u32>,
    uplink_view: UplinkView,
    uplink: UplinkRuntime,
}

impl LanRuntime {
    pub(super) fn new() -> Self {
        Self {
            settings: LanSettings::default(),
            settings_error: None,
            presence: None,
            presence_error: None,
            last_start_attempt: None,
            facts: Vec::new(),
            facts_error: None,
            last_facts_at: None,
            sightings: HashMap::new(),
            proofs: HashMap::new(),
            authenticator: crate::share::lan_presence_auth::LanAuthenticator::new(),
            authenticated_contacts: Vec::new(),
            link_facts: Vec::new(),
            uplink_pins: Vec::new(),
            evidence_revision: None,
            reported: HashMap::new(),
            reported_hashes: Vec::new(),
            unknown_devices: 0,
            shared_ifaces: Vec::new(),
            uplink_view: UplinkView::default(),
            uplink: UplinkRuntime::new(),
        }
    }

    /// Stop an active sharing session synchronously (daemon shutdown).
    pub(super) fn shutdown(&mut self) {
        self.link_facts.clear();
        let _ = crate::share::lan_uplink_evidence::publish(&[]);
        self.uplink.shutdown();
        if let Some(presence) = self.presence.take() {
            presence.withdraw();
        }
    }

    /// Interfaces confirmed by current pinned status channels.
    pub(super) fn peer_ifaces(&self, now: i64) -> Vec<u32> {
        let mut out: Vec<_> = self
            .link_facts
            .iter()
            .filter(|fact| fact.fresh(now))
            .map(|fact| fact.interface.index)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Only current channel answers, without mDNS subnet estimates.
    pub(super) fn paired_sightings(&self, now: i64) -> Vec<(String, bool, Vec<u32>)> {
        self.link_facts
            .iter()
            .filter(|fact| fact.fresh(now) && fact.can_share_on(&self.facts, &self.shared_ifaces))
            .map(|fact| {
                (
                    lan_presence_match::hashed_lan_id(&fact.pin.node_id),
                    fact.peer_uplink,
                    vec![fact.interface.index],
                )
            })
            .collect()
    }

    /// Parent hands this RAM-only snapshot to ShareService after the tick.
    pub(super) fn link_host_facts(&self) -> LanLinkHostFacts {
        let known = self.settings_error.is_none()
            && self.facts_error.is_none()
            && self
                .last_facts_at
                .is_some_and(|at| at.elapsed() <= FACTS_INTERVAL + Duration::from_secs(1));
        let interfaces = if known {
            self.facts.clone()
        } else {
            Vec::new()
        };
        let own_uplink = if known && !interfaces.is_empty() {
            OwnUplink::from_interfaces(
                &interfaces,
                self.uplink.internet_verdict_snapshot(),
                &self.peer_ifaces(crate::share::core_now_secs()),
                &self.shared_ifaces,
            )
        } else {
            OwnUplink::Unknown
        };
        LanLinkHostFacts {
            enabled: self.settings.presence_enabled && self.settings_error.is_none(),
            interfaces,
            shared_ifaces: self.shared_ifaces.clone(),
            own_uplink,
        }
    }

    pub(super) fn classified_links(&self) -> Vec<(InterfaceFacts, LinkClass)> {
        crate::net::classify_links(&self.facts, &[], None, &self.shared_ifaces)
    }

    /// "I have my own internet" as announced to peers: an uplink-class link
    /// that is not the link the peers were seen on.
    pub(super) fn uplink_advisory(&self, now: i64) -> bool {
        let peer_ifaces = self.peer_ifaces(now);
        crate::net::classify_links(&self.facts, &peer_ifaces, None, &self.shared_ifaces)
            .iter()
            .any(|(_, class)| *class == LinkClass::Uplink)
    }

    pub(super) fn tick(&mut self, input: LanTickInput<'_>) -> Vec<ShareEvent> {
        self.refresh_settings();
        self.refresh_facts();
        self.link_facts = if self.settings.presence_enabled
            && self.settings_error.is_none()
            && self.facts_error.is_none()
        {
            input
                .paired_links
                .iter()
                .take(lan_link_facts::MAX_LINK_PEERS)
                .filter(|fact| fact.current(input.now, input.contacts, input.grants, &self.facts))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        self.link_facts.sort_by(|left, right| {
            left.pin
                .node_id
                .cmp(&right.pin.node_id)
                .then(left.connection_id.cmp(&right.connection_id))
        });
        let mut events = Vec::new();
        if !self.settings.presence_enabled {
            if self.presence.take().is_some() {
                self.sightings.clear();
                self.proofs.clear();
                self.authenticated_contacts.clear();
                for contact_id in self.reported.keys() {
                    events.push(ShareEvent::LanPeerLost {
                        contact_id: contact_id.clone(),
                    });
                }
                self.reported.clear();
                self.reported_hashes.clear();
            }
            let _ = self.authenticator.refresh(&[], None);
        } else {
            if let Err(error) = self
                .authenticator
                .refresh(input.contacts, input.own_node_id)
            {
                self.presence_error = Some(error);
            }
            self.ensure_started();
            self.refresh_announcement(&input);
            self.drain_presence_events();
            self.expire_sightings(input.now);
            events.extend(self.reconcile(input.contacts, input.now));
        }
        let revision = if self.settings.uplink_sharing_enabled {
            self.link_facts.clone()
        } else {
            Vec::new()
        };
        if self.evidence_revision.as_ref() != Some(&revision) {
            match crate::share::lan_uplink_evidence::publish(&revision) {
                Ok(()) => self.evidence_revision = Some(revision),
                Err(error) => {
                    self.presence_error = Some(error);
                    self.link_facts.clear();
                }
            }
        }
        let revoked = self
            .uplink_pins
            .iter()
            .any(|pin| !lan_link_facts::pin_current(pin, input.contacts, input.grants));
        let authority_lost = revoked
            && self.shared_ifaces.iter().any(|index| {
                !self
                    .link_facts
                    .iter()
                    .any(|fact| fact.interface.index == *index)
            });
        self.tick_uplink(input.own_node_id, input.now, authority_lost);
        if self.shared_ifaces.is_empty() {
            self.uplink_pins.clear();
        }
        for fact in &self.link_facts {
            if self.uplink_pins.len() < lan_link_facts::MAX_LINK_PEERS
                && !self.uplink_pins.contains(&fact.pin)
            {
                self.uplink_pins.push(fact.pin.clone());
            }
        }
        events
    }

    /// Feed the uplink policy with the current links and paired sightings.
    fn tick_uplink(&mut self, own_node_id: Option<&str>, now: i64, authority_lost: bool) {
        let peer_ifaces = self.peer_ifaces(now);
        let peers: Vec<PeerOnLink> = self
            .paired_sightings(now)
            .into_iter()
            .map(|(hashed_id, uplink, ifaces)| PeerOnLink {
                hashed_id,
                uplink,
                ifaces,
                authenticated: true,
            })
            .collect();
        let own_hashed_id = own_node_id
            .map(lan_presence_match::hashed_lan_id)
            .unwrap_or_default();
        let mut settings = self.settings.clone();
        if self.settings_error.is_some() || !settings.presence_enabled || authority_lost {
            settings.uplink_sharing_enabled = false;
        }
        let facts = self.facts.clone();
        let view = self.uplink.tick(UplinkTickInput {
            settings: &settings,
            facts: &facts,
            peer_ifaces: &peer_ifaces,
            peers: &peers,
            own_hashed_id: &own_hashed_id,
            now,
        });
        self.shared_ifaces = self.uplink.shared_ifaces();
        self.uplink_view = view;
    }

    fn refresh_settings(&mut self) {
        match LanSettings::load() {
            Ok(settings) => {
                self.settings = settings;
                self.settings_error = None;
            }
            Err(error) => self.settings_error = Some(error),
        }
    }

    fn refresh_facts(&mut self) {
        let due = self
            .last_facts_at
            .is_none_or(|at| at.elapsed() >= FACTS_INTERVAL);
        if !due {
            return;
        }
        self.last_facts_at = Some(Instant::now());
        match crate::net::gather_interface_facts() {
            Ok(mut facts) => {
                self.uplink.refine_facts(&mut facts);
                self.facts = facts;
                self.facts_error = None;
            }
            Err(error) => self.facts_error = Some(error),
        }
    }

    fn ensure_started(&mut self) {
        if self.presence.is_some() {
            return;
        }
        let retry_due = self
            .last_start_attempt
            .is_none_or(|at| at.elapsed() >= START_RETRY);
        if !retry_due {
            return;
        }
        self.last_start_attempt = Some(Instant::now());
        match LanPresence::start() {
            Ok(presence) => {
                self.presence = Some(presence);
                self.presence_error = None;
            }
            Err(error) => self.presence_error = Some(error),
        }
    }

    pub(super) fn status(&self, contacts: &[DirectContact], now: i64) -> LanStatus {
        let presence = if let Some(error) = &self.settings_error {
            LanFacility::Unavailable(error.clone())
        } else if !self.settings.presence_enabled {
            LanFacility::Disabled
        } else if let Some(error) = &self.presence_error {
            LanFacility::Unavailable(error.clone())
        } else if self.presence.is_some() {
            LanFacility::Available
        } else {
            LanFacility::Starting
        };
        let announced_id = self
            .presence
            .as_ref()
            .and_then(|presence| presence.announced())
            .map(|announcement| announcement.hashed_id);
        let peer_ifaces = self.peer_ifaces(now);
        let peers = contacts
            .iter()
            .filter(|contact| lan_presence_match::lan_evidence_current(contact, now))
            .map(|contact| LanPeerView {
                contact_id: contact.id.clone(),
                display_name: contact.display_name.clone(),
                candidates: contact.lan_candidates.clone(),
                uplink: self.link_facts.iter().find(|fact| fact.fresh(now)
                    && matches!(&fact.pin.origin, lan_link_facts::PinOrigin::Contact { id, .. } if id == &contact.id))
                    .map(|fact| fact.peer_uplink),
                seen_at: contact.lan_seen_at.unwrap_or_default(),
            })
            .collect();
        let links = self
            .classified_links()
            .into_iter()
            .filter(|(facts, _)| !facts.loopback)
            .map(|(facts, class)| LinkView {
                name: facts.name.clone(),
                index: facts.index,
                class: class.label().to_string(),
                addrs: facts.addrs.iter().map(ToString::to_string).collect(),
                has_gateway: facts.has_gateway,
                dhcp_lease: facts.dhcp_lease,
                peer_present: peer_ifaces.contains(&facts.index),
            })
            .collect();
        LanStatus {
            presence,
            announced_id,
            peers,
            links,
            links_error: self.facts_error.clone(),
            unknown_devices: self.unknown_devices,
            uplink: self.uplink_view.clone(),
        }
    }
}
