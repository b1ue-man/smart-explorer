//! Host-side credential snapshot for private, signed LAN announcements.
use std::collections::HashSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::identity::code_renewal_registry::retiring_lookups;
use super::lan_presence::RetiringLanAnnouncement;
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

pub struct LanAuthenticator {
    identity: Option<ShareIdentity>,
    principals: Vec<Principal>,
    audience: Vec<(String, String, String)>,
    refreshed_at: Option<Instant>,
    own_node: Option<String>,
    ids: std::collections::HashMap<String, usize>,
    /// Contacts whose own announcement arrived sealed (`v=3`) and verified:
    /// they read `v=3`. While one audience contact is not among them, the
    /// plain `v=2` record is announced next to the sealed one.
    sealed_readers: Mutex<HashSet<String>>,
}

impl LanAuthenticator {
    pub fn new() -> Self {
        Self {
            identity: None,
            principals: Vec::new(),
            audience: Vec::new(),
            refreshed_at: None,
            own_node: None,
            ids: std::collections::HashMap::new(),
            sealed_readers: Mutex::new(HashSet::new()),
        }
    }

    pub fn refresh(
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
        if let Ok(mut readers) = self.sealed_readers.lock() {
            readers.retain(|id| audience.iter().any(|(contact_id, _, _)| contact_id == id));
        }
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

    pub fn announcement(
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
        let mut proof = lan_privacy::make_proof(
            &sighting,
            epoch,
            now.saturating_add(lan_privacy::PROOF_LIFETIME_SECS),
            &identity.iroh_secret,
        );
        proof.sealed = lan_privacy::seal_proof(&sighting, &proof, &identity.direct_secret);
        let all_read_sealed = self.sealed_readers.lock().is_ok_and(|readers| {
            self.principals
                .iter()
                .all(|principal| readers.contains(&principal.contact_id))
        });
        let retiring = retiring_announcements(identity, &sighting, epoch, now);
        Some((
            LanAnnouncement {
                hashed_id: id,
                p4: sighting.p4,
                p6: sighting.p6,
                uplink,
                plain_v2: proof.sealed.is_none() || !all_read_sealed,
                retiring,
            },
            proof,
        ))
    }

    /// The contact a sighting verifies for, and the sighting cut to the addresses
    /// its signed list names (E26). Addresses the contact did not sign are dropped.
    pub fn authenticate_cut(
        &self,
        sighting: &LanSighting,
        proof: &LanProof,
        now: i64,
    ) -> Option<(String, LanSighting)> {
        let principal = self.principals.get(*self.ids.get(&sighting.id)?)?;
        let accepted = lan_privacy::accepted_any_sighting(
            sighting,
            proof,
            &principal.node,
            &principal.secret,
            now,
        )?;
        if proof.sealed.is_some() {
            if let Ok(mut readers) = self.sealed_readers.lock() {
                readers.insert(principal.contact_id.clone());
            }
        }
        Some((principal.contact_id.clone(), accepted))
    }

    pub fn authenticate(
        &self,
        sighting: &LanSighting,
        proof: &LanProof,
        now: i64,
    ) -> Option<String> {
        self.authenticate_cut(sighting, proof, now)
            .map(|(contact_id, _)| contact_id)
    }
}

/// E17: the same facts under the rotating id and seal of every lookup id
/// that runs out after a Direct-code renewal.
fn retiring_announcements(
    identity: &ShareIdentity,
    current: &LanSighting,
    epoch: i64,
    now: i64,
) -> Vec<RetiringLanAnnouncement> {
    retiring_lookups(&identity.node_id)
        .into_iter()
        .filter_map(|(_, secret)| {
            let id = lan_privacy::rotating_id(&identity.node_id, &secret, epoch)?;
            let sighting = LanSighting {
                id: id.clone(),
                ..current.clone()
            };
            let mut proof = lan_privacy::make_proof(
                &sighting,
                epoch,
                now.saturating_add(lan_privacy::PROOF_LIFETIME_SECS),
                &identity.iroh_secret,
            );
            proof.sealed = lan_privacy::seal_proof(&sighting, &proof, &secret);
            Some(RetiringLanAnnouncement {
                hashed_id: id,
                proof,
            })
        })
        .collect()
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

#[cfg(test)]
#[path = "lan_presence_auth_task_tests.rs"]
pub(crate) mod task_tests;

#[cfg(test)]
#[path = "lan_kuerzung_wahl_task_tests.rs"]
mod kuerzung_wahl_task_tests;
