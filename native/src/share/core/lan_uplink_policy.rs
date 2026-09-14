//! Decides when this host should share its internet uplink with paired
//! devices on a router-less link, and when to stop again. Pure: every input
//! is a typed fact, every output a decision the daemon applies.
use crate::net::{InterfaceFacts, LinkClass};

pub const START_DEBOUNCE_SECS: i64 = 5;
pub const STOP_GRACE_SECS: i64 = 90;
pub const UPLINK_LOSS_SECS: i64 = 15;

/// One paired peer seen on the LAN: its hashed id, its advisory "I have my
/// own internet" flag, and the local interfaces it was seen on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerOnLink {
    pub hashed_id: String,
    pub uplink: bool,
    pub ifaces: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyInput<'a> {
    pub enabled: bool,
    pub adapter_available: bool,
    pub links: &'a [(InterfaceFacts, LinkClass)],
    pub peers: &'a [PeerOnLink],
    pub own_hashed_id: &'a str,
    pub stop_requested: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Keep,
    Start { private_if: u32, public_if: u32 },
    Stop(String),
    Idle(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UplinkPolicy {
    sharing: Option<(u32, u32)>,
    candidate_since: Option<(u32, u32, i64)>,
    peer_absent_since: Option<i64>,
    uplink_lost_since: Option<i64>,
}

impl UplinkPolicy {
    pub fn sharing(&self) -> Option<(u32, u32)> {
        self.sharing
    }

    /// Adopt a sharing session that was found active at daemon start.
    pub fn resume(&mut self, private_if: u32, public_if: u32) {
        self.sharing = Some((private_if, public_if));
        self.candidate_since = None;
        self.peer_absent_since = None;
        self.uplink_lost_since = None;
    }

    pub fn mark_started(&mut self, private_if: u32, public_if: u32) {
        self.resume(private_if, public_if);
    }

    pub fn mark_stopped(&mut self) {
        self.sharing = None;
        self.candidate_since = None;
        self.peer_absent_since = None;
        self.uplink_lost_since = None;
    }

    pub fn evaluate(&mut self, input: &PolicyInput<'_>, now: i64) -> Decision {
        if !input.enabled {
            return self.stop_or_idle("Internet-Teilen ist ausgeschaltet");
        }
        if !input.adapter_available {
            return self.stop_or_idle("Internet-Teilen ist auf diesem System nicht verfuegbar");
        }
        if input.stop_requested && self.sharing.is_some() {
            return self.stop_now("auf Wunsch beendet");
        }
        let uplinks: Vec<u32> = input
            .links
            .iter()
            .filter(|(_, class)| *class == LinkClass::Uplink)
            .map(|(facts, _)| facts.index)
            .collect();
        let router_less: Vec<u32> = input
            .links
            .iter()
            .filter(|(_, class)| *class == LinkClass::RouterLess)
            .map(|(facts, _)| facts.index)
            .collect();

        if let Some((private_if, public_if)) = self.sharing {
            let peer_present = input
                .peers
                .iter()
                .any(|peer| peer.ifaces.contains(&private_if));
            if peer_present {
                self.peer_absent_since = None;
            } else if self.peer_absent_since.is_none() {
                self.peer_absent_since = Some(now);
            }
            if uplinks.contains(&public_if) {
                self.uplink_lost_since = None;
            } else if self.uplink_lost_since.is_none() {
                self.uplink_lost_since = Some(now);
            }
            if self
                .peer_absent_since
                .is_some_and(|since| now.saturating_sub(since) >= STOP_GRACE_SECS)
            {
                return self.stop_now("kein gekoppeltes Geraet mehr auf dem Link");
            }
            if self
                .uplink_lost_since
                .is_some_and(|since| now.saturating_sub(since) >= UPLINK_LOSS_SECS)
            {
                return self.stop_now("eigener Internetzugang verloren");
            }
            return Decision::Keep;
        }

        // Not sharing: look for a router-less link with a paired peer that
        // lacks its own internet, and exactly this host having an uplink.
        let mut candidate: Option<(u32, u32)> = None;
        let mut reason = String::new();
        for private_if in &router_less {
            let peers_here: Vec<&PeerOnLink> = input
                .peers
                .iter()
                .filter(|peer| peer.ifaces.contains(private_if))
                .collect();
            if peers_here.is_empty() {
                continue;
            }
            if peers_here.iter().any(|peer| peer.uplink) {
                // Both sides have internet: the lexicographically smaller
                // hashed id would share; report instead of flapping.
                let smallest_peer = peers_here
                    .iter()
                    .map(|peer| peer.hashed_id.as_str())
                    .min()
                    .unwrap_or("");
                if !uplinks.is_empty() && input.own_hashed_id < smallest_peer {
                    reason = "Peer hat eigenen Internetzugang; dieses Geraet teilt nicht".into();
                } else {
                    reason = "Peer hat eigenen Internetzugang".into();
                }
                continue;
            }
            let Some(public_if) = uplinks.iter().find(|index| *index != private_if) else {
                reason = "kein eigener Internetzugang".into();
                continue;
            };
            candidate = Some((*private_if, *public_if));
            break;
        }
        let Some((private_if, public_if)) = candidate else {
            self.candidate_since = None;
            if reason.is_empty() {
                reason = if router_less.is_empty() {
                    "kein Link ohne Router".into()
                } else {
                    "kein gekoppeltes Geraet auf einem Link ohne Router".into()
                };
            }
            return Decision::Idle(reason);
        };
        match self.candidate_since {
            Some((p, u, since)) if p == private_if && u == public_if => {
                if now.saturating_sub(since) >= START_DEBOUNCE_SECS {
                    Decision::Start {
                        private_if,
                        public_if,
                    }
                } else {
                    Decision::Idle("gekoppeltes Geraet erkannt; Start folgt".into())
                }
            }
            _ => {
                self.candidate_since = Some((private_if, public_if, now));
                Decision::Idle("gekoppeltes Geraet erkannt; Start folgt".into())
            }
        }
    }

    fn stop_or_idle(&mut self, reason: &str) -> Decision {
        if self.sharing.is_some() {
            self.stop_now(reason)
        } else {
            self.candidate_since = None;
            Decision::Idle(reason.into())
        }
    }

    fn stop_now(&mut self, reason: &str) -> Decision {
        Decision::Stop(reason.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(index: u32, class: LinkClass) -> (InterfaceFacts, LinkClass) {
        (
            InterfaceFacts {
                name: format!("if{index}"),
                adapter_id: format!("id{index}"),
                index,
                up: true,
                loopback: false,
                addrs: Vec::new(),
                has_gateway: class == LinkClass::Uplink,
                dhcp_lease: None,
            },
            class,
        )
    }

    fn peer(iface: u32, uplink: bool) -> PeerOnLink {
        PeerOnLink {
            hashed_id: "bbbb".into(),
            uplink,
            ifaces: vec![iface],
        }
    }

    fn input<'a>(
        links: &'a [(InterfaceFacts, LinkClass)],
        peers: &'a [PeerOnLink],
        enabled: bool,
    ) -> PolicyInput<'a> {
        PolicyInput {
            enabled,
            adapter_available: true,
            links,
            peers,
            own_hashed_id: "aaaa",
            stop_requested: false,
        }
    }

    #[test]
    fn lan_cleanup_task_starts_after_debounce_and_stops_after_grace() {
        let links = vec![link(2, LinkClass::RouterLess), link(3, LinkClass::Uplink)];
        let peers = vec![peer(2, false)];
        let mut policy = UplinkPolicy::default();
        assert!(matches!(policy.evaluate(&input(&links, &peers, true), 100), Decision::Idle(_)));
        assert!(matches!(policy.evaluate(&input(&links, &peers, true), 102), Decision::Idle(_)));
        assert_eq!(
            policy.evaluate(&input(&links, &peers, true), 106),
            Decision::Start {
                private_if: 2,
                public_if: 3
            }
        );
        policy.mark_started(2, 3);
        assert_eq!(policy.evaluate(&input(&links, &peers, true), 110), Decision::Keep);
        let none: Vec<PeerOnLink> = Vec::new();
        assert_eq!(policy.evaluate(&input(&links, &none, true), 120), Decision::Keep);
        assert!(matches!(
            policy.evaluate(&input(&links, &none, true), 120 + STOP_GRACE_SECS),
            Decision::Stop(_)
        ));
    }

    #[test]
    fn lan_cleanup_task_idle_reasons_cover_missing_uplink_peer_and_disabled() {
        let links = vec![link(2, LinkClass::RouterLess)];
        let peers = vec![peer(2, false)];
        let mut policy = UplinkPolicy::default();
        assert_eq!(
            policy.evaluate(&input(&links, &peers, true), 1),
            Decision::Idle("kein eigener Internetzugang".into())
        );
        let links = vec![link(2, LinkClass::RouterLess), link(3, LinkClass::Uplink)];
        assert_eq!(
            policy.evaluate(&input(&links, &[], true), 1),
            Decision::Idle("kein gekoppeltes Geraet auf einem Link ohne Router".into())
        );
        assert_eq!(
            policy.evaluate(&input(&links, &peers, false), 1),
            Decision::Idle("Internet-Teilen ist ausgeschaltet".into())
        );
        let mut unavailable = input(&links, &peers, true);
        unavailable.adapter_available = false;
        assert!(matches!(policy.evaluate(&unavailable, 1), Decision::Idle(_)));
    }

    #[test]
    fn lan_cleanup_task_both_sides_with_internet_do_not_share() {
        let links = vec![link(2, LinkClass::RouterLess), link(3, LinkClass::Uplink)];
        let peers = vec![peer(2, true)];
        let mut policy = UplinkPolicy::default();
        assert!(matches!(policy.evaluate(&input(&links, &peers, true), 10), Decision::Idle(_)));
        assert!(matches!(policy.evaluate(&input(&links, &peers, true), 20), Decision::Idle(_)));
    }

    #[test]
    fn lan_cleanup_task_uplink_loss_stop_request_and_disable_stop_sharing() {
        let links = vec![link(2, LinkClass::RouterLess), link(3, LinkClass::Uplink)];
        let peers = vec![peer(2, false)];
        let mut policy = UplinkPolicy::default();
        policy.resume(2, 3);
        let lost = vec![link(2, LinkClass::RouterLess), link(3, LinkClass::Routed)];
        assert_eq!(policy.evaluate(&input(&lost, &peers, true), 50), Decision::Keep);
        assert!(matches!(
            policy.evaluate(&input(&lost, &peers, true), 50 + UPLINK_LOSS_SECS),
            Decision::Stop(_)
        ));
        policy.mark_stopped();
        policy.resume(2, 3);
        let mut stop = input(&links, &peers, true);
        stop.stop_requested = true;
        assert_eq!(policy.evaluate(&stop, 60), Decision::Stop("auf Wunsch beendet".into()));
        policy.resume(2, 3);
        assert!(matches!(policy.evaluate(&input(&links, &peers, false), 70), Decision::Stop(_)));
    }
}
