//! Host-side credential snapshot for private, signed LAN announcements.
use std::time::{Duration, Instant};

use super::lan_privacy::{self, LanProof};
use super::{
    DirectAccessState, DirectContact, LanAnnouncement, LanSighting, ShareIdentity, ShareProfiles,
};

struct Principal {
    contact_id: String,
    node: String,
    secret: Vec<u8>,
}

impl Drop for Principal {
    fn drop(&mut self) {
        self.secret.fill(0);
    }
}

pub(crate) struct LanAuthenticator {
    identity: Option<ShareIdentity>,
    principals: Vec<Principal>,
    audience: Vec<(String, String, String)>,
    refreshed_at: Option<Instant>,
    own_node: Option<String>,
    ids: std::collections::HashMap<String, usize>,
}

impl LanAuthenticator {
    pub(crate) fn new() -> Self {
        Self {
            identity: None,
            principals: Vec::new(),
            audience: Vec::new(),
            refreshed_at: None,
            own_node: None,
            ids: std::collections::HashMap::new(),
        }
    }

    pub(crate) fn refresh(
        &mut self,
        contacts: &[DirectContact],
        own_node: Option<&str>,
    ) -> Result<(), String> {
        let audience: Vec<_> = contacts
            .iter()
            .filter(|contact| {
                contact.access_state == DirectAccessState::Accepted
                    && contact.remote_device_id.is_some()
            })
            .filter_map(|contact| {
                node(contact).map(|node| (contact.id.clone(), node, contact.lookup_id.clone()))
            })
            .collect();
        if audience.is_empty() || own_node.is_none() {
            self.identity = None;
            self.principals.clear();
            self.ids.clear();
            self.audience = audience;
            self.refreshed_at = None;
            self.own_node = own_node.map(str::to_owned);
            return Ok(());
        }
        let due = self.audience != audience
            || self.own_node.as_deref() != own_node
            || self
                .refreshed_at
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(60));
        if !due {
            return Ok(());
        }
        // Invalidate first: a failed credential refresh cannot retain a
        // removed contact or manufacture a fallback stable announcement.
        self.identity = None;
        self.principals.clear();
        self.ids.clear();
        self.audience = audience;
        self.refreshed_at = Some(Instant::now());
        self.own_node = own_node.map(str::to_owned);
        let identity = ShareIdentity::load_or_create(String::new())?;
        if Some(identity.node_id.as_str()) != own_node {
            return Err("LAN-Identitaet hat sich geaendert; Ankuendigung wartet".into());
        }
        for contact in contacts.iter().filter(|contact| {
            contact.access_state == DirectAccessState::Accepted
                && contact.remote_device_id.is_some()
        }) {
            let Some(node) = node(contact) else {
                continue;
            };
            let Some(secret) = ShareProfiles::direct_secret_checked(contact)? else {
                continue;
            };
            if secret.len() == 32 {
                self.principals.push(Principal {
                    contact_id: contact.id.clone(),
                    node,
                    secret,
                });
            }
        }
        self.identity = Some(identity);
        let epoch = super::core_now_secs().div_euclid(lan_privacy::ID_EPOCH_SECS);
        for (index, principal) in self.principals.iter().enumerate() {
            for epoch in [epoch.saturating_sub(1), epoch, epoch.saturating_add(1)] {
                if let Some(id) =
                    lan_privacy::rotating_id(&principal.node, &principal.secret, epoch)
                {
                    self.ids.insert(id, index);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn announcement(
        &self,
        ports: (Option<u16>, Option<u16>),
        uplink: bool,
        addresses: Vec<std::net::IpAddr>,
        now: i64,
    ) -> Option<(LanAnnouncement, LanProof)> {
        let identity = self.identity.as_ref()?;
        if self.principals.is_empty() || (ports.0.is_none() && ports.1.is_none()) {
            return None;
        }
        let epoch = now.div_euclid(lan_privacy::ID_EPOCH_SECS);
        let id = lan_privacy::rotating_id(&identity.node_id, &identity.direct_secret, epoch)?;
        let mut addresses: Vec<_> = addresses
            .into_iter()
            .filter(|ip| !ip.is_loopback() && !ip.is_unspecified() && !ip.is_multicast())
            .collect();
        addresses.sort();
        addresses.dedup();
        addresses.truncate(lan_privacy::MAX_ADDRESSES);
        if addresses.is_empty() {
            return None;
        }
        let sighting = LanSighting {
            id: id.clone(),
            addrs: addresses,
            p4: ports.0.unwrap_or(0),
            p6: ports.1.unwrap_or(0),
            uplink,
            seen_at: now,
        };
        let proof = lan_privacy::make_proof(
            &sighting,
            epoch,
            now.saturating_add(lan_privacy::PROOF_LIFETIME_SECS),
            &identity.iroh_secret,
        );
        Some((
            LanAnnouncement {
                hashed_id: id,
                p4: sighting.p4,
                p6: sighting.p6,
                uplink,
            },
            proof,
        ))
    }

    pub(crate) fn authenticate(
        &self,
        sighting: &LanSighting,
        proof: &LanProof,
        now: i64,
    ) -> Option<String> {
        let principal = self.principals.get(*self.ids.get(&sighting.id)?)?;
        lan_privacy::verify_sighting(sighting, proof, &principal.node, &principal.secret, now)
            .then(|| principal.contact_id.clone())
    }
}

fn node(contact: &DirectContact) -> Option<String> {
    if !contact.expected_node_id.is_empty() {
        Some(contact.expected_node_id.clone())
    } else {
        contact
            .remote_public_key
            .clone()
            .or_else(|| contact.accepted_public_key.clone())
    }
}
