//! Matching local-network announcements to paired Direct contacts.
//!
//! An announcement carries only a hashed endpoint id and ports; it proves
//! nothing. Matching it to a contact merely tells the dialer where to try.
//! Identity is proven later by the Iroh TLS node pin and the relation's
//! session proof, exactly as for server-delivered presence.
use std::net::IpAddr;

use sha2::{Digest, Sha256};

use super::core::now_secs;
use super::types::{DirectAccessState, DirectContact, PeerPresence};

/// Announcements refresh every 60 s; evidence older than this is dropped.
pub const LAN_PRESENCE_TTL_SECS: i64 = 150;
pub const LAN_PRESENCE_VERSION: &str = "1";
const LAN_ID_DOMAIN: &str = "se-lan-presence-v1";

/// Stable per-endpoint id that does not reveal the Iroh key itself.
pub fn hashed_lan_id(node_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(LAN_ID_DOMAIN.as_bytes());
    hasher.update(b"|");
    hasher.update(node_id.trim().as_bytes());
    let digest = hasher.finalize();
    digest[..8].iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanSighting {
    pub id: String,
    pub addrs: Vec<IpAddr>,
    pub p4: u16,
    pub p6: u16,
    pub uplink: bool,
    pub seen_at: i64,
}

/// The hashed id a contact's device would announce, from its pinned node
/// (or its pinned key when the node pin is still empty).
pub fn contact_lan_id(contact: &DirectContact) -> Option<String> {
    let node = if !contact.expected_node_id.trim().is_empty() {
        contact.expected_node_id.clone()
    } else {
        contact
            .remote_public_key
            .clone()
            .or_else(|| contact.accepted_public_key.clone())?
    };
    Some(hashed_lan_id(&node))
}

/// Dial candidates for a sighting: IPv4 as `ip:p4`, global IPv6 as
/// `[ip]:p6`, link-local IPv6 once per local interface scope.
pub fn candidates_for(sighting: &LanSighting, local_scopes: &[u32]) -> Vec<String> {
    let mut out = Vec::new();
    for ip in &sighting.addrs {
        match ip {
            IpAddr::V4(v4) => {
                if sighting.p4 != 0 && !v4.is_loopback() && !v4.is_unspecified() {
                    out.push(format!("{v4}:{}", sighting.p4));
                }
            }
            IpAddr::V6(v6) => {
                if sighting.p6 == 0 || v6.is_loopback() || v6.is_unspecified() {
                    continue;
                }
                if (v6.segments()[0] & 0xffc0) == 0xfe80 {
                    for scope in local_scopes {
                        out.push(crate::net::scoped_v6_candidate(*v6, sighting.p6, *scope));
                    }
                } else {
                    out.push(format!("[{v6}]:{}", sighting.p6));
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The accepted contact an announcement belongs to, with its dial candidates.
pub fn match_sighting(
    contacts: &[DirectContact],
    sighting: &LanSighting,
    local_scopes: &[u32],
) -> Option<(String, Vec<String>)> {
    let contact = contacts.iter().find(|contact| {
        contact.access_state == DirectAccessState::Accepted
            && contact.remote_device_id.is_some()
            && contact_lan_id(contact).as_deref() == Some(sighting.id.as_str())
    })?;
    let candidates = candidates_for(sighting, local_scopes);
    if candidates.is_empty() {
        return None;
    }
    Some((contact.id.clone(), candidates))
}

/// Local interfaces a sighting plausibly arrived on: the ones sharing an
/// IPv4 link-local or /24 prefix with an announced address, or — for a peer
/// announcing only IPv6 link-local addresses — every router-less link.
pub fn peer_interfaces(
    sighting: &LanSighting,
    links: &[(crate::net::InterfaceFacts, crate::net::LinkClass)],
) -> Vec<u32> {
    let mut out = Vec::new();
    let v4_peer: Vec<std::net::Ipv4Addr> = sighting
        .addrs
        .iter()
        .filter_map(|ip| match ip {
            IpAddr::V4(v4) if !v4.is_loopback() => Some(*v4),
            _ => None,
        })
        .collect();
    for (facts, class) in links {
        if !facts.up || facts.loopback {
            continue;
        }
        let same_prefix = facts.addrs.iter().any(|local| match local {
            IpAddr::V4(local) => v4_peer.iter().any(|peer| {
                let local = local.octets();
                let peer = peer.octets();
                (local[0] == 169 && local[1] == 254 && peer[0] == 169 && peer[1] == 254)
                    || local[..3] == peer[..3]
            }),
            IpAddr::V6(_) => false,
        });
        let v6_only_link_local = v4_peer.is_empty()
            && sighting.addrs.iter().all(|ip| crate::net::is_link_local(ip))
            && *class == crate::net::LinkClass::RouterLess;
        if same_prefix || v6_only_link_local {
            out.push(facts.index);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

pub fn lan_evidence_current(contact: &DirectContact, now: i64) -> bool {
    contact
        .lan_seen_at
        .is_some_and(|seen| seen <= now && seen.saturating_add(LAN_PRESENCE_TTL_SECS) >= now)
        && !contact.lan_candidates.is_empty()
}

/// Routing evidence for dialing: the current server presence with LAN
/// candidates prepended, or a presence synthesized from the contact's pins
/// when only LAN evidence exists. `None` when the peer is not reachable.
pub fn effective_presence(contact: &DirectContact, now: i64) -> Option<PeerPresence> {
    let lan = lan_evidence_current(contact, now);
    let current_server = contact
        .presence
        .as_ref()
        .filter(|presence| presence.is_current_at(now));
    match (current_server, lan) {
        (Some(presence), true) => {
            let mut merged = presence.clone();
            let mut candidates = contact.lan_candidates.clone();
            for existing in &presence.candidates {
                if !candidates.contains(existing) {
                    candidates.push(existing.clone());
                }
            }
            merged.candidates = candidates;
            Some(merged)
        }
        (Some(presence), false) => Some(presence.clone()),
        (None, true) => {
            let device_id = contact.remote_device_id.clone()?;
            let public_key = contact
                .remote_public_key
                .clone()
                .or_else(|| contact.accepted_public_key.clone())?;
            let node_id = if contact.expected_node_id.trim().is_empty() {
                public_key.clone()
            } else {
                contact.expected_node_id.clone()
            };
            Some(PeerPresence {
                kind: "direct".into(),
                relation_id: contact.lookup_id.clone(),
                device_id,
                device_name: contact.display_name.clone(),
                public_key,
                fingerprint: contact.expected_fingerprint.clone(),
                node_id,
                relay_url: String::new(),
                candidates: contact.lan_candidates.clone(),
                expires_at: contact
                    .lan_seen_at
                    .unwrap_or_else(now_secs)
                    .saturating_add(LAN_PRESENCE_TTL_SECS),
                nonce: "lan".into(),
                proof: String::new(),
            })
        }
        (None, false) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::types::ShareStatus;

    fn contact(node: &str, accepted: bool) -> DirectContact {
        DirectContact {
            id: format!("contact-{node}"),
            display_name: format!("Device {node}"),
            lookup_id: format!("lookup-{node}"),
            expected_fingerprint: "fp".into(),
            expected_node_id: node.into(),
            remote_device_id: Some(format!("device-{node}")),
            remote_public_key: Some(node.into()),
            auto_connect: true,
            auto_open: false,
            last_seen: None,
            status: ShareStatus::Offline,
            last_error: None,
            presence: None,
            access_state: if accepted {
                DirectAccessState::Accepted
            } else {
                DirectAccessState::Pending
            },
            request_sent_at: None,
            accepted_at: None,
            accepted_public_key: None,
            lan_candidates: Vec::new(),
            lan_seen_at: None,
            lan_uplink: None,
        }
    }

    fn sighting(id: &str, addrs: &[&str]) -> LanSighting {
        LanSighting {
            id: id.into(),
            addrs: addrs.iter().map(|ip| ip.parse().unwrap()).collect(),
            p4: 4000,
            p6: 6000,
            uplink: false,
            seen_at: 100,
        }
    }

    #[test]
    fn lan_cleanup_task_hashed_ids_are_stable_short_and_not_the_key() {
        let id = hashed_lan_id("node-key");
        assert_eq!(id.len(), 16);
        assert_eq!(id, hashed_lan_id(" node-key "));
        assert_ne!(id, hashed_lan_id("node-key2"));
        assert!(!id.contains("node"));
    }

    #[test]
    fn lan_cleanup_task_only_accepted_contacts_with_a_device_match() {
        let accepted = contact("a", true);
        let pending = contact("b", false);
        let contacts = vec![accepted.clone(), pending];
        let seen = sighting(&hashed_lan_id("a"), &["169.254.1.2"]);
        let (contact_id, candidates) = match_sighting(&contacts, &seen, &[]).unwrap();
        assert_eq!(contact_id, "contact-a");
        assert_eq!(candidates, ["169.254.1.2:4000"]);
        let unmatched = sighting(&hashed_lan_id("b"), &["169.254.1.3"]);
        assert!(match_sighting(&contacts, &unmatched, &[]).is_none());
        assert!(match_sighting(&contacts, &sighting("unknown", &["10.0.0.1"]), &[]).is_none());
    }

    #[test]
    fn lan_cleanup_task_candidates_cover_v4_global_v6_and_scoped_link_local() {
        let seen = sighting("x", &["169.254.1.2", "fe80::1", "2001:db8::5", "127.0.0.1"]);
        let candidates = candidates_for(&seen, &[3, 7]);
        assert_eq!(
            candidates,
            [
                "169.254.1.2:4000",
                "[2001:db8::5]:6000",
                "[fe80::1%3]:6000",
                "[fe80::1%7]:6000",
            ]
        );
    }

    #[test]
    fn lan_cleanup_task_peer_interfaces_follow_shared_prefixes() {
        use crate::net::{InterfaceFacts, LinkClass};
        let links = vec![
            (
                InterfaceFacts {
                    name: "eth0".into(),
                    adapter_id: "eth0".into(),
                    index: 2,
                    up: true,
                    loopback: false,
                    addrs: vec!["169.254.7.7".parse().unwrap()],
                    has_gateway: false,
                    dhcp_lease: Some(false),
                },
                LinkClass::RouterLess,
            ),
            (
                InterfaceFacts {
                    name: "wlan0".into(),
                    adapter_id: "wlan0".into(),
                    index: 3,
                    up: true,
                    loopback: false,
                    addrs: vec!["192.168.1.20".parse().unwrap()],
                    has_gateway: true,
                    dhcp_lease: Some(true),
                },
                LinkClass::Uplink,
            ),
        ];
        assert_eq!(peer_interfaces(&sighting("x", &["169.254.1.2"]), &links), [2]);
        assert_eq!(peer_interfaces(&sighting("x", &["192.168.1.30"]), &links), [3]);
        assert_eq!(peer_interfaces(&sighting("x", &["fe80::1"]), &links), [2]);
        assert!(peer_interfaces(&sighting("x", &["10.9.9.9"]), &links).is_empty());
    }

    #[test]
    fn lan_cleanup_task_effective_presence_merges_or_synthesizes() {
        let mut c = contact("a", true);
        assert!(effective_presence(&c, 100).is_none());
        c.lan_candidates = vec!["169.254.1.2:4000".into()];
        c.lan_seen_at = Some(90);
        let synthesized = effective_presence(&c, 100).expect("lan evidence");
        assert_eq!(synthesized.node_id, "a");
        assert_eq!(synthesized.candidates, ["169.254.1.2:4000"]);
        assert!(synthesized.is_current_at(100));
        assert!(effective_presence(&c, 90 + LAN_PRESENCE_TTL_SECS + 1).is_none());

        c.presence = Some(PeerPresence {
            kind: "direct".into(),
            relation_id: "lookup-a".into(),
            device_id: "device-a".into(),
            device_name: "Device a".into(),
            public_key: "a".into(),
            fingerprint: "fp".into(),
            node_id: "a".into(),
            relay_url: "https://relay".into(),
            candidates: vec!["203.0.113.4:4000".into()],
            expires_at: 500,
            nonce: "n".into(),
            proof: "p".into(),
        });
        let merged = effective_presence(&c, 100).expect("server presence");
        assert_eq!(merged.candidates, ["169.254.1.2:4000", "203.0.113.4:4000"]);
        assert_eq!(merged.relay_url, "https://relay");
    }
}
