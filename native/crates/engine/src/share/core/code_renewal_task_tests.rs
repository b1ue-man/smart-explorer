//! Task suite of the Direct-code renewal (E17, M21), cases 1, 3 and 5: the
//! pure half. State machine, identity half and profile half run on memory
//! stores; nothing reads or writes a file, the secure store or the network.
use std::collections::HashMap;

use super::*;
use crate::share::identity::{direct_secret_account, IdentityPersistence, ShareIdentity};
use crate::share::profiles::ShareProfiles;
use crate::share::types::{
    DirectAccessState, DirectContact, DirectGrant, DirectGrantState, ShareStatus,
};

#[derive(Default)]
struct MemoryStorage {
    identity: Option<String>,
    secrets: HashMap<String, String>,
}

impl IdentityPersistence for MemoryStorage {
    fn load_identity(&mut self) -> Result<Option<String>, String> {
        Ok(self.identity.clone())
    }

    fn save_identity(&mut self, contents: &str) -> Result<(), String> {
        self.identity = Some(contents.to_string());
        Ok(())
    }

    fn load_secret(&mut self, account: &str) -> Result<Option<String>, String> {
        Ok(self.secrets.get(account).cloned())
    }

    fn save_secret(&mut self, account: &str, secret: &str) -> Result<(), String> {
        self.secrets.insert(account.to_string(), secret.to_string());
        Ok(())
    }

    fn delete_secret(&mut self, account: &str) -> Result<(), String> {
        self.secrets.remove(account);
        Ok(())
    }
}

fn peer(device: &str) -> RenewalPeer {
    RenewalPeer {
        device_id: device.to_string(),
        device_name: format!("Gerät {device}"),
        public_key: format!("key-{device}"),
        node_id: format!("node-{device}"),
        legacy: false,
    }
}

fn pending_ids(code: &RetiringCode) -> Vec<&str> {
    code.pending
        .iter()
        .map(|peer| peer.device_id.as_str())
        .collect()
}

fn contact(id: &str, device: &str) -> DirectContact {
    DirectContact {
        id: id.into(),
        display_name: format!("Kontakt {id}"),
        lookup_id: format!("lookup-{id}"),
        expected_fingerprint: format!("fp-{device}"),
        expected_node_id: format!("node-{device}"),
        remote_device_id: Some(device.into()),
        remote_public_key: Some(format!("key-{device}")),
        auto_connect: true,
        auto_open: false,
        last_seen: None,
        status: ShareStatus::Offline,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: Some(1),
        accepted_public_key: Some(format!("key-{device}")),
        lan_candidates: Vec::new(),
        lan_seen_at: None,
        lan_uplink: None,
        relation: Default::default(),
    }
}

fn grant(device: &str, state: DirectGrantState) -> DirectGrant {
    DirectGrant {
        device_id: device.into(),
        device_name: format!("Gerät {device}"),
        public_key: format!("key-{device}"),
        fingerprint: format!("fp-{device}"),
        node_id: format!("node-{device}"),
        state,
        updated_at: 1,
        exec: Default::default(),
        write: false,
    }
}

#[test]
fn code_renewal_task_need_follows_the_latest_removal() {
    let mut state = CodeRenewalState::default();
    assert_eq!(state.need(Some(10)), RenewalNeed::Initialize(10));
    assert_eq!(state.need(None), RenewalNeed::Initialize(0));
    state.initialize(10);
    assert_eq!(state.need(Some(10)), RenewalNeed::Nothing);
    assert_eq!(state.need(Some(9)), RenewalNeed::Nothing);
    assert_eq!(state.need(None), RenewalNeed::Nothing);
    assert_eq!(state.need(Some(11)), RenewalNeed::Renew(11));
    // A removal before the first run of this version renews nothing
    // retroactively; only a later one does.
    assert_eq!(state.covered_removals_at, Some(10));
    // The next renewal waits while every allowed generation (MAX_RETIRING_CODES) still runs out.
    for index in 0..MAX_RETIRING_CODES {
        state.retiring.push(RetiringCode {
            lookup_id: format!("old-{index}"),
            generation: index as u64 + 1,
            started_at: 1,
            pending: vec![peer("p")],
        });
    }
    assert_eq!(state.need(Some(11)), RenewalNeed::Deferred);
    assert_eq!(state.need(Some(10)), RenewalNeed::Nothing);
}

#[test]
fn code_renewal_task_removal_allocates_new_data_and_keeps_the_old_secret() {
    let mut storage = MemoryStorage::default();
    let identity = ShareIdentity::load_or_create_with("Gerät".into(), &mut storage).unwrap();
    let old_lookup = identity.direct_lookup_id.clone();
    let old_account = direct_secret_account(&old_lookup);
    let renewed = identity.allocate_renewed_with(&mut storage).unwrap();
    assert_ne!(renewed.direct_lookup_id, old_lookup);
    assert_ne!(renewed.direct_secret, identity.direct_secret);
    assert_eq!(renewed.device_id, identity.device_id);
    assert_eq!(renewed.node_id, identity.node_id);
    assert_eq!(renewed.public_key, identity.public_key);
    assert_eq!(renewed.fingerprint, identity.fingerprint);
    // Phase one: the secure store has the new secret, no durable file names it.
    assert!(storage
        .secrets
        .contains_key(&direct_secret_account(&renewed.direct_lookup_id)));
    let unchanged = ShareIdentity::load_or_create_with("Gerät".into(), &mut storage).unwrap();
    assert_eq!(unchanged.direct_lookup_id, old_lookup);
    // Phase two: the identity file names the new data and writes no cleanup
    // marker, so no grant is suspended.
    renewed.commit_renewed_with(&mut storage).unwrap();
    assert!(ShareIdentity::pending_cleanup_action_with(&mut storage)
        .unwrap()
        .is_none());
    let current = ShareIdentity::load_or_create_with("Gerät".into(), &mut storage).unwrap();
    assert_eq!(current.direct_lookup_id, renewed.direct_lookup_id);
    assert_eq!(current.direct_secret, renewed.direct_secret);
    // The old account stays until its holders confirmed.
    assert!(storage.secrets.contains_key(&old_account));
    let retiring = ShareIdentity::retiring_secret_with(&mut storage, &old_lookup).unwrap();
    assert_eq!(retiring, Some(identity.direct_secret));
    ShareIdentity::delete_retiring_secret_with(&mut storage, &old_lookup).unwrap();
    assert!(!storage.secrets.contains_key(&old_account));
    let gone = ShareIdentity::retiring_secret_with(&mut storage, &old_lookup).unwrap();
    assert_eq!(gone, None);
}

#[test]
fn code_renewal_task_removed_contact_leaves_the_holders_and_the_withdrawn_keys() {
    let mut profiles = ShareProfiles::default();
    for (id, device) in [("cx", "x"), ("cp", "p"), ("cq", "q"), ("cr", "r")] {
        profiles.direct_contacts.push(contact(id, device));
    }
    profiles
        .direct_grants
        .push(grant("x", DirectGrantState::Accepted));
    profiles
        .direct_grants
        .push(grant("p", DirectGrantState::Accepted));
    profiles
        .direct_grants
        .push(grant("q", DirectGrantState::Reconfirm));
    profiles
        .direct_grants
        .push(grant("r", DirectGrantState::Ignored));
    assert_eq!(profiles.latest_direct_removal(), None);
    assert!(profiles.withdrawn_direct_keys().is_empty());
    assert!(profiles.forget_direct_peer("cx", 50).is_some());
    let holders = profiles.direct_code_holders();
    let devices: Vec<&str> = holders
        .iter()
        .map(|holder| holder.device_id.as_str())
        .collect();
    // Active and suspended grants may come back with the old code; a block
    // and the removed device never.
    assert_eq!(devices, ["p", "q"]);
    assert_eq!(holders[0].public_key, "key-p");
    assert_eq!(holders[0].node_id, "node-p");
    assert_eq!(profiles.latest_direct_removal(), Some(50));
    assert_eq!(profiles.withdrawn_direct_keys(), ["node-x", "key-x"]);
    assert!(profiles.holds_direct_code(&peer("p")));
    assert!(profiles.holds_direct_code(&peer("q")));
    assert!(!profiles.holds_direct_code(&peer("r")));
    assert!(!profiles.holds_direct_code(&peer("x")));
}

#[test]
fn code_renewal_task_prepare_then_settle_completes_or_rolls_back() {
    let mut state = CodeRenewalState::default();
    state.initialize(10);
    state.prepare("alt", "neu", vec![peer("p"), peer("q")], Some(11), 100);
    assert!(state.prepared.is_some());
    assert_eq!(state.generation, 1);
    // The identity file names the new data: the entry stays, the removal is
    // answered and the old secret is not deleted.
    let mut completed = state.clone();
    assert!(completed.settle("neu").is_empty());
    assert!(completed.prepared.is_none());
    assert_eq!(completed.covered_removals_at, Some(11));
    assert_eq!(completed.retiring.len(), 1);
    assert_eq!(completed.retiring[0].lookup_id, "alt");
    assert_eq!(completed.retiring[0].generation, 1);
    assert_eq!(completed.retiring[0].started_at, 100);
    assert_eq!(pending_ids(&completed.retiring[0]), ["p", "q"]);
    // The identity file was not replaced: the new secret is an orphan, the
    // entry of the still current data goes, the removal stays unanswered.
    let mut rolled_back = state.clone();
    assert_eq!(rolled_back.settle("alt"), ["neu"]);
    assert!(rolled_back.prepared.is_none());
    assert!(rolled_back.retiring.is_empty());
    assert_eq!(rolled_back.covered_removals_at, Some(10));
}

#[test]
fn code_renewal_task_renewal_without_holders_ends_at_once() {
    let mut state = CodeRenewalState::default();
    state.initialize(10);
    state.prepare("alt", "neu", Vec::new(), Some(11), 100);
    assert_eq!(state.settle("neu"), ["alt"]);
    assert!(state.retiring.is_empty());
    assert_eq!(state.covered_removals_at, Some(11));
    assert!(!state.status(false, None).active);
}

#[test]
fn code_renewal_task_generations_move_a_holder_only_forward() {
    let mut state = CodeRenewalState::default();
    state.initialize(1);
    state.prepare("alt", "neu", vec![peer("p")], Some(2), 100);
    assert!(state.settle("neu").is_empty());
    // A second renewal: p still holds the oldest data, so it stays in the
    // first entry and the entry of `neu` waits for q only.
    state.prepare("neu", "neu2", vec![peer("p"), peer("q")], Some(3), 200);
    assert!(state.settle("neu2").is_empty());
    assert_eq!(state.generation, 2);
    assert_eq!(state.retiring.len(), 2);
    assert_eq!(state.retiring[0].lookup_id, "alt");
    assert_eq!(pending_ids(&state.retiring[0]), ["p"]);
    assert_eq!(state.retiring[1].lookup_id, "neu");
    assert_eq!(state.retiring[1].generation, 2);
    assert_eq!(pending_ids(&state.retiring[1]), ["q"]);
    let (key, node) = ("key-p", "node-p");

    // p took the current data over: it leaves its entry, the first
    // generation ends, the second is untouched.
    let mut current = state.clone();
    assert_eq!(current.confirm("p", key, node, "neu2", "neu2"), ["alt"]);
    assert_eq!(current.retiring.len(), 1);
    assert_eq!(pending_ids(&current.retiring[0]), ["q"]);

    // p took the younger retiring data over: it moves to that entry.
    let mut moved = state.clone();
    assert_eq!(moved.confirm("p", key, node, "neu", "neu2"), ["alt"]);
    assert_eq!(moved.retiring.len(), 1);
    assert_eq!(moved.retiring[0].lookup_id, "neu");
    assert_eq!(pending_ids(&moved.retiring[0]), ["q", "p"]);

    // Data nobody knows change nothing.
    let mut unknown = state.clone();
    assert!(unknown
        .confirm("p", key, node, "unbekannt", "neu2")
        .is_empty());
    assert_eq!(unknown, state);

    // Never back to an older generation, never for a pin that differs.
    let mut backwards = state.clone();
    assert!(backwards
        .confirm("q", "key-q", "node-q", "alt", "neu2")
        .is_empty());
    assert_eq!(backwards, state);
    let mut foreign = state.clone();
    assert!(foreign
        .confirm("p", "key-other", node, "neu2", "neu2")
        .is_empty());
    assert_eq!(foreign, state);
}

#[test]
fn code_renewal_task_old_peer_keeps_its_entry_for_good() {
    let mut state = CodeRenewalState::default();
    state.initialize(1);
    state.prepare("alt", "neu", vec![peer("p")], Some(2), 100);
    assert!(state.settle("neu").is_empty());
    assert!(!state.status(false, None).deferred);
    // The peer used the old data without renewal support: it stays pending,
    // flagged, and the entry stays.
    assert!(state.mark_legacy("p", "key-p", "node-p", "alt"));
    assert!(!state.mark_legacy("p", "key-p", "node-p", "alt"));
    assert!(!state.mark_legacy("p", "key-p", "node-p", "unbekannt"));
    assert!(!state.mark_legacy("q", "key-q", "node-q", "alt"));
    let status = state.status(false, None);
    assert!(status.active);
    assert_eq!(status.pending_devices, 1);
    assert_eq!(status.legacy_devices, 1);
    assert_eq!(status.pending_names, ["Gerät p"]);
    assert_eq!(status.since, Some(100));
    // As long as it holds a grant, nothing ends the entry.
    assert!(state
        .prune(|candidate| candidate.device_id == "p")
        .is_empty());
    assert_eq!(state.retiring.len(), 1);
    assert!(state.retiring[0].pending[0].legacy);
    // Only a lost grant ends it.
    assert_eq!(state.prune(|_| false), ["alt"]);
    assert!(state.retiring.is_empty());
    assert!(!state.status(false, None).active);
}
