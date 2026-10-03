//! Authority of the small paired LAN status channel; never filesystem rights.
use std::net::{IpAddr, SocketAddr};

use serde::{Deserialize, Serialize};

use super::direct_protocol::DirectPeerIdentity;
use super::types::{DirectAccessState, DirectContact, DirectGrant, DirectGrantState};
use crate::net::InterfaceFacts;

pub(crate) const MAX_LINK_PEERS: usize = 32;
pub(crate) const MAX_FACT_LIFETIME_SECS: i64 = 8;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", deny_unknown_fields)]
pub(crate) enum PinOrigin {
    Contact { id: String, lookup_id: String },
    Grant { device_id: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct LanPeerPin {
    pub(crate) origin: PinOrigin,
    pub(crate) device_id: String,
    pub(crate) public_key: String,
    pub(crate) fingerprint: String,
    pub(crate) node_id: String,
}

impl LanPeerPin {
    pub(crate) fn identity(&self) -> DirectPeerIdentity {
        DirectPeerIdentity {
            device_id: self.device_id.clone(),
            device_name: String::new(),
            public_key: self.public_key.clone(),
            fingerprint: self.fingerprint.clone(),
            node_id: self.node_id.clone(),
        }
    }

    pub(crate) fn matches_tls_identity(
        &self,
        identity: &DirectPeerIdentity,
        tls_remote: &str,
    ) -> bool {
        identity.device_name.is_empty()
            && identity.validate().is_ok()
            && self.node_id == tls_remote
            && self.identity() == *identity
    }
}

pub(crate) fn contact_pin(contact: &DirectContact) -> Option<LanPeerPin> {
    if contact.access_state != DirectAccessState::Accepted {
        return None;
    }
    let key = contact
        .remote_public_key
        .as_ref()
        .or(contact.accepted_public_key.as_ref())?;
    if contact
        .remote_public_key
        .as_ref()
        .zip(contact.accepted_public_key.as_ref())
        .is_some_and(|(remote, accepted)| remote != accepted)
    {
        return None;
    }
    if contact.id.is_empty() || contact.id.len() > 256 || contact.lookup_id.len() > 256 {
        return None;
    }
    let pin = LanPeerPin {
        origin: PinOrigin::Contact {
            id: contact.id.clone(),
            lookup_id: contact.lookup_id.clone(),
        },
        device_id: contact.remote_device_id.clone()?,
        public_key: key.clone(),
        fingerprint: contact.expected_fingerprint.clone(),
        node_id: if contact.expected_node_id.is_empty() {
            key.clone()
        } else {
            contact.expected_node_id.clone()
        },
    };
    pin.identity().validate().ok()?;
    Some(pin)
}

pub(crate) fn grant_pin(grant: &DirectGrant) -> Option<LanPeerPin> {
    if grant.state != DirectGrantState::Accepted {
        return None;
    }
    let pin = LanPeerPin {
        origin: PinOrigin::Grant {
            device_id: grant.device_id.clone(),
        },
        device_id: grant.device_id.clone(),
        public_key: grant.public_key.clone(),
        fingerprint: grant.fingerprint.clone(),
        node_id: if grant.node_id.is_empty() {
            grant.public_key.clone()
        } else {
            grant.node_id.clone()
        },
    };
    pin.identity().validate().ok()?;
    Some(pin)
}

pub(crate) fn pin_current(
    pin: &LanPeerPin,
    contacts: &[DirectContact],
    grants: &[DirectGrant],
) -> bool {
    match &pin.origin {
        PinOrigin::Contact { id, .. } => contacts
            .iter()
            .filter(|contact| &contact.id == id)
            .filter_map(contact_pin)
            .any(|current| current == *pin),
        PinOrigin::Grant { device_id } => grants
            .iter()
            .filter(|grant| &grant.device_id == device_id)
            .filter_map(grant_pin)
            .any(|current| current == *pin),
    }
}

pub(crate) fn accepted_pin_for_tls(
    contacts: &[DirectContact],
    grants: &[DirectGrant],
    identity: &DirectPeerIdentity,
    remote: &str,
) -> Option<LanPeerPin> {
    contacts
        .iter()
        .filter_map(contact_pin)
        .chain(grants.iter().filter_map(grant_pin))
        .find(|pin| pin.matches_tls_identity(identity, remote))
}

/// A missing platform verdict remains unknown, even if no gateway was listed.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OwnUplink {
    Unknown,
    Present,
    Absent,
}

impl OwnUplink {
    pub(crate) fn known(self) -> Option<bool> {
        match self {
            Self::Unknown => None,
            Self::Present => Some(true),
            Self::Absent => Some(false),
        }
    }

    pub(crate) fn from_interfaces(
        facts: &[InterfaceFacts],
        internet: Option<&[u32]>,
        peers: &[u32],
        shared: &[u32],
    ) -> Self {
        let Some(internet) = internet else {
            return Self::Unknown;
        };
        if facts.is_empty()
            || internet
                .iter()
                .any(|index| !facts.iter().any(|iface| iface.index == *index))
        {
            return Self::Unknown;
        }
        // A positive OS verdict does not require guessing from a gateway.
        let own = facts.iter().any(|iface| {
            iface.up
                && !iface.loopback
                && !iface.addrs.is_empty()
                && internet.contains(&iface.index)
                && !peers.contains(&iface.index)
                && !shared.contains(&iface.index)
        });
        if own {
            Self::Present
        } else {
            Self::Absent
        }
    }
}

#[derive(Clone)]
pub(crate) struct LanLinkHostFacts {
    pub(crate) enabled: bool,
    pub(crate) interfaces: Vec<InterfaceFacts>,
    pub(crate) shared_ifaces: Vec<u32>,
    pub(crate) own_uplink: OwnUplink,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrivateInterface {
    pub(crate) index: u32,
    pub(crate) adapter_id: String,
    pub(crate) name: String,
    pub(crate) local_ip: IpAddr,
}

pub(crate) fn private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_link_local(),
        IpAddr::V6(ip) => {
            (ip.segments()[0] & 0xfe00) == 0xfc00 || (ip.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Match the actual selected socket's local IP, never an announced subnet.
pub(crate) fn private_interface(
    local: IpAddr,
    remote: SocketAddr,
    facts: &[InterfaceFacts],
) -> Option<PrivateInterface> {
    if !private_ip(local)
        || !private_ip(remote.ip())
        || remote.port() == 0
        || local.is_ipv4() != remote.is_ipv4()
    {
        return None;
    }
    let mut matching = facts.iter().filter(|iface| iface.addrs.contains(&local));
    let iface = matching.next()?;
    if matching.next().is_some()
        || !iface.up
        || iface.loopback
        || iface.index == 0
        || iface.adapter_id.is_empty()
        || iface.adapter_id.len() > 512
        || iface.name.len() > 512
    {
        return None;
    }
    if let SocketAddr::V6(remote) = remote {
        if remote.scope_id() != 0 && remote.scope_id() != iface.index {
            return None;
        }
    }
    Some(PrivateInterface {
        index: iface.index,
        adapter_id: iface.adapter_id.clone(),
        name: iface.name.clone(),
        local_ip: local,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthenticatedLanFact {
    pub(crate) pin: LanPeerPin,
    pub(crate) interface: PrivateInterface,
    pub(crate) remote_addr: SocketAddr,
    pub(crate) path_id: String,
    pub(crate) connection_id: u64,
    pub(crate) challenge: [u8; 32],
    pub(crate) peer_uplink: bool,
    pub(crate) confirmed_at: i64,
    pub(crate) expires_at: i64,
}

impl AuthenticatedLanFact {
    pub(crate) fn fresh(&self, now: i64) -> bool {
        self.confirmed_at > 0
            && now >= self.confirmed_at
            && now < self.expires_at
            && self.expires_at > self.confirmed_at
            && self.expires_at.saturating_sub(self.confirmed_at) <= MAX_FACT_LIFETIME_SECS
            && !self.path_id.is_empty()
            && self.path_id.len() <= 128
            && self.challenge.iter().any(|byte| *byte != 0)
    }

    pub(crate) fn current(
        &self,
        now: i64,
        contacts: &[DirectContact],
        grants: &[DirectGrant],
        facts: &[InterfaceFacts],
    ) -> bool {
        self.fresh(now)
            && pin_current(&self.pin, contacts, grants)
            && private_interface(self.interface.local_ip, self.remote_addr, facts)
                .is_some_and(|current| current == self.interface)
    }

    /// A status channel survives DHCP received from the sharing peer. Giving
    /// NAT/DHCP authority still requires a known router-less local interface.
    pub(crate) fn can_share_on(&self, facts: &[InterfaceFacts], shared_ifaces: &[u32]) -> bool {
        facts
            .iter()
            .find(|iface| {
                iface.index == self.interface.index
                    && iface.adapter_id == self.interface.adapter_id
                    && iface.addrs.contains(&self.interface.local_ip)
            })
            .is_some_and(|iface| {
                shared_ifaces.contains(&iface.index)
                    || (!iface.has_gateway && iface.dhcp_lease == Some(false))
            })
    }
}

#[cfg(test)]
#[path = "lan_link_task_tests.rs"]
mod task_tests;
