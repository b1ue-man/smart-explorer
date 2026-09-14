//! Daemon-side owner of local-network presence: starts the mDNS announcer
//! when enabled, keeps the announcement in sync with the Iroh ports and the
//! uplink verdict, turns sightings of paired peers into `ShareEvent`s, expires
//! stale evidence, and renders the `LanStatus` snapshot for GUI and CLI.
use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::net::{InterfaceFacts, LinkClass};
use crate::share::lan_uplink_policy::PeerOnLink;
use crate::share::{
    lan_presence_match, DirectContact, LanAnnouncement, LanEvent, LanFacility, LanPeerView,
    LanPresence, LanSettings, LanSighting, LanStatus, LinkView, ShareEvent, UplinkView,
    LAN_PRESENCE_TTL_SECS,
};

use super::lan_uplink_runtime::{UplinkRuntime, UplinkTickInput};

const FACTS_INTERVAL: Duration = Duration::from_secs(5);
const START_RETRY: Duration = Duration::from_secs(60);

pub(super) struct LanTickInput<'a> {
    pub(super) contacts: &'a [DirectContact],
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
        self.uplink.shutdown();
        if let Some(presence) = self.presence.take() {
            presence.withdraw();
        }
    }

    /// Interfaces where a paired peer was seen recently.
    pub(super) fn peer_ifaces(&self, now: i64) -> Vec<u32> {
        let links = self.classified_links();
        let mut out = Vec::new();
        for sighting in self.sightings.values() {
            if sighting.seen_at.saturating_add(LAN_PRESENCE_TTL_SECS) < now {
                continue;
            }
            if !self.reported_hashes.contains(&sighting.id) {
                continue;
            }
            out.extend(lan_presence_match::peer_interfaces(sighting, &links));
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Paired peers seen recently, with the advisory uplink flag.
    pub(super) fn paired_sightings(&self, now: i64) -> Vec<(String, bool, Vec<u32>)> {
        let links = self.classified_links();
        self.sightings
            .values()
            .filter(|sighting| sighting.seen_at.saturating_add(LAN_PRESENCE_TTL_SECS) >= now)
            .filter(|sighting| self.reported_hashes.contains(&sighting.id))
            .map(|sighting| {
                (
                    sighting.id.clone(),
                    sighting.uplink,
                    lan_presence_match::peer_interfaces(sighting, &links),
                )
            })
            .collect()
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
        let mut events = Vec::new();
        if !self.settings.presence_enabled {
            if self.presence.take().is_some() {
                self.sightings.clear();
                for contact_id in self.reported.keys() {
                    events.push(ShareEvent::LanPeerLost {
                        contact_id: contact_id.clone(),
                    });
                }
                self.reported.clear();
                self.reported_hashes.clear();
            }
        } else {
            self.ensure_started();
            self.refresh_announcement(&input);
            self.drain_presence_events();
            self.expire_sightings(input.now);
            events.extend(self.reconcile(input.contacts, input.now));
        }
        self.tick_uplink(input.own_node_id, input.now);
        events
    }

    /// Feed the uplink policy with the current links and paired sightings.
    fn tick_uplink(&mut self, own_node_id: Option<&str>, now: i64) {
        let peer_ifaces = self.peer_ifaces(now);
        let peers: Vec<PeerOnLink> = self
            .paired_sightings(now)
            .into_iter()
            .map(|(hashed_id, uplink, ifaces)| PeerOnLink {
                hashed_id,
                uplink,
                ifaces,
            })
            .collect();
        let own_hashed_id = own_node_id
            .map(lan_presence_match::hashed_lan_id)
            .unwrap_or_default();
        let settings = self.settings.clone();
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

    fn refresh_announcement(&mut self, input: &LanTickInput<'_>) {
        let Some(presence) = &self.presence else {
            return;
        };
        let (Some(node_id), (p4, p6)) = (input.own_node_id, input.ports) else {
            presence.withdraw();
            return;
        };
        if p4.is_none() && p6.is_none() {
            presence.withdraw();
            return;
        }
        let announcement = LanAnnouncement {
            hashed_id: lan_presence_match::hashed_lan_id(node_id),
            p4: p4.unwrap_or(0),
            p6: p6.unwrap_or(0),
            uplink: input.uplink_advisory,
        };
        if let Err(error) = presence.announce(&announcement) {
            self.presence_error = Some(error);
        }
    }

    fn drain_presence_events(&mut self) {
        let Some(presence) = &self.presence else {
            return;
        };
        let own = presence.announced().map(|announcement| announcement.hashed_id);
        let drained: Vec<LanEvent> = presence.events().try_iter().collect();
        for event in drained {
            match event {
                LanEvent::Seen(sighting) => {
                    if own.as_deref() == Some(sighting.id.as_str()) {
                        continue;
                    }
                    self.sightings.insert(sighting.id.clone(), sighting);
                }
                LanEvent::Lost(id) => {
                    self.sightings.remove(&id);
                }
                LanEvent::Error(error) => self.presence_error = Some(error),
            }
        }
    }

    fn expire_sightings(&mut self, now: i64) {
        self.sightings
            .retain(|_, sighting| sighting.seen_at.saturating_add(LAN_PRESENCE_TTL_SECS) >= now);
    }

    /// Compare the current sightings with what was reported last time and
    /// emit the difference as events.
    fn reconcile(&mut self, contacts: &[DirectContact], now: i64) -> Vec<ShareEvent> {
        let links = self.classified_links();
        let scopes: Vec<u32> = links
            .iter()
            .filter(|(facts, class)| *class != LinkClass::Inactive && facts.index != 0)
            .map(|(facts, _)| facts.index)
            .collect();
        let mut seen_now: HashMap<String, (Vec<String>, bool)> = HashMap::new();
        let mut seen_hashes = Vec::new();
        let mut unknown = 0usize;
        for sighting in self.sightings.values() {
            match lan_presence_match::match_sighting(contacts, sighting, &scopes) {
                Some((contact_id, candidates)) => {
                    seen_now.insert(contact_id, (candidates, sighting.uplink));
                    seen_hashes.push(sighting.id.clone());
                }
                None => unknown += 1,
            }
        }
        self.unknown_devices = unknown;
        self.reported_hashes = seen_hashes;
        let mut events = Vec::new();
        for (contact_id, (candidates, uplink)) in &seen_now {
            let unchanged = self
                .reported
                .get(contact_id)
                .is_some_and(|(previous, previous_uplink)| {
                    previous == candidates && previous_uplink == uplink
                });
            let stale = contacts
                .iter()
                .find(|contact| &contact.id == contact_id)
                .is_some_and(|contact| {
                    contact
                        .lan_seen_at
                        .is_none_or(|seen| seen.saturating_add(LAN_PRESENCE_TTL_SECS / 2) < now)
                });
            if !unchanged || stale {
                events.push(ShareEvent::LanPeerSeen {
                    contact_id: contact_id.clone(),
                    candidates: candidates.clone(),
                    uplink: *uplink,
                });
            }
        }
        for contact_id in self.reported.keys() {
            if !seen_now.contains_key(contact_id) {
                events.push(ShareEvent::LanPeerLost {
                    contact_id: contact_id.clone(),
                });
            }
        }
        self.reported = seen_now;
        events
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
                uplink: contact.lan_uplink,
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
