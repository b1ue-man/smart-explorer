//! Task suite of the Direct-code renewal (E17, M21), cases 2 (e), 3, 4 and 5
//! (c): which sessions and old requests the host accepts while a lookup id
//! runs out, and when it stops. Sessions run through
//! `authenticate_incoming_session` on memory state; the process view of the
//! renewal (`code_renewal_registry`) is keyed by a node id of its own per
//! test and reset when the test ends.
use std::io;
use std::sync::{Arc, Mutex};

use zeroize::Zeroizing;

use super::{CodeRenewalState, RenewalPeer};
use crate::share::core::{hmac_proof, presence_payload, public_fingerprint};
use crate::share::identity::code_renewal_registry::{self as registry, RetiringMaterial};
use crate::share::identity::ShareIdentity;
use crate::share::profiles::ShareProfiles;
use crate::share::session::denial::{denial_of, SessionDenial};
use crate::share::session::{authenticate_incoming_session, session_payload};
use crate::share::types::{
    DirectGrant, DirectGrantState, DirectRequestPolicy, PeerPresence, ShareAuthState,
};
use crate::share::wire::PeerHello;

const ALT: &str = "cr-task-fall3-alt";
const NEU: &str = "cr-task-fall3-neu";
const ALT_SECRET: u8 = 31;
const NEU_SECRET: u8 = 32;

/// Empties the process view of one host when the test ends, also on a panic.
struct ResetRegistry(String);

impl Drop for ResetRegistry {
    fn drop(&mut self) {
        registry::install(&self.0, "", Vec::new(), Vec::new(), Default::default());
    }
}

fn identity(device_id: &str, seed: u8, lookup_id: &str, secret: u8) -> ShareIdentity {
    let key = iroh::SecretKey::from_bytes(&[seed; 32]);
    let node_id = key.public().to_string();
    ShareIdentity {
        device_id: device_id.into(),
        device_name: device_id.into(),
        direct_lookup_id: lookup_id.into(),
        public_key: node_id.clone(),
        fingerprint: public_fingerprint(node_id.as_bytes()),
        node_id,
        iroh_secret: key,
        direct_secret: [secret; 32],
    }
}

fn grant(client: &ShareIdentity, state: DirectGrantState) -> DirectGrant {
    DirectGrant {
        device_id: client.device_id.clone(),
        device_name: client.device_name.clone(),
        public_key: client.public_key.clone(),
        fingerprint: client.fingerprint.clone(),
        node_id: client.node_id.clone(),
        state,
        updated_at: 1,
        exec: Default::default(),
        write: false,
    }
}

fn renewal_peer(device: &ShareIdentity) -> RenewalPeer {
    RenewalPeer {
        device_id: device.device_id.clone(),
        device_name: device.device_name.clone(),
        public_key: device.public_key.clone(),
        node_id: device.node_id.clone(),
        legacy: false,
    }
}

/// The host after a renewal: its current data are `NEU`; `p` and `q` hold a
/// grant; the process view lists `ALT` as running out for `pending`.
struct Scene {
    host: ShareIdentity,
    p: ShareIdentity,
    q: ShareIdentity,
    x: ShareIdentity,
    auth: Arc<Mutex<ShareAuthState>>,
    _reset: ResetRegistry,
}

impl Scene {
    fn new(seed: u8) -> Self {
        let host = identity("host", seed, NEU, NEU_SECRET);
        let p = identity("device-p", seed + 1, "lookup-p", 41);
        let q = identity("device-q", seed + 2, "lookup-q", 42);
        let x = identity("device-x", seed + 3, "lookup-x", 43);
        let auth = Arc::new(Mutex::new(ShareAuthState {
            identity: host.clone(),
            direct_secret: host.direct_secret(),
            default_direct_exports: Default::default(),
            direct_contacts: Vec::new(),
            direct_grants: vec![
                grant(&p, DirectGrantState::Accepted),
                grant(&q, DirectGrantState::Accepted),
            ],
            rooms: Vec::new(),
            direct_requests: Vec::new(),
            direct_request_tombstones: Vec::new(),
            seen_nonces: Default::default(),
            direct_online: true,
            authorization_epoch: 1,
        }));
        let reset = ResetRegistry(host.node_id.clone());
        Self {
            host,
            p,
            q,
            x,
            auth,
            _reset: reset,
        }
    }

    /// The process view the durable store installs for `state`.
    fn install(&self, state: &CodeRenewalState) {
        let retiring = state
            .retiring
            .iter()
            .map(|code| {
                assert_eq!(
                    code.lookup_id, ALT,
                    "the scene knows the secret of ALT only"
                );
                RetiringMaterial {
                    lookup_id: code.lookup_id.clone(),
                    secret: Zeroizing::new(vec![ALT_SECRET; 32]),
                    pending: code
                        .pending
                        .iter()
                        .map(|peer| {
                            (
                                peer.device_id.clone(),
                                peer.public_key.clone(),
                                peer.node_id.clone(),
                            )
                        })
                        .collect(),
                }
            })
            .collect();
        registry::install(
            &self.host.node_id,
            NEU,
            retiring,
            Vec::new(),
            state.status(false, None),
        );
    }

    /// Whether a Direct hello of `client` naming `relation`, proven with
    /// `secret`, is accepted: `Err(Some(class))` is a rejection the peer may
    /// learn, `Err(None)` one without a class.
    fn hello(
        &self,
        client: &ShareIdentity,
        relation: &str,
        secret: u8,
    ) -> Result<(), Option<SessionDenial>> {
        let nonce = "renewal-nonce";
        let payload = session_payload(
            "direct",
            relation,
            &client.device_id,
            &self.host.device_id,
            &client.node_id,
            &self.host.node_id,
            nonce,
        );
        let hello = PeerHello {
            protocol_version: 3,
            relation_kind: "direct".into(),
            relation_id: relation.into(),
            device_id: client.device_id.clone(),
            public_key: client.public_key.clone(),
            node_id: client.node_id.clone(),
            session_nonce: nonce.into(),
            session_proof: hmac_proof(&[secret; 32], &payload),
            requested_capabilities: Vec::new(),
        };
        authenticate_incoming_session(&hello, &client.node_id, &self.auth)
            .map(|_| ())
            .map_err(|error: io::Error| denial_of(&error))
    }
}

fn running_out_for_p_and_q(scene: &Scene) -> CodeRenewalState {
    let mut state = CodeRenewalState::default();
    state.initialize(1);
    state.prepare(
        ALT,
        NEU,
        vec![renewal_peer(&scene.p), renewal_peer(&scene.q)],
        Some(2),
        100,
    );
    assert!(state.settle(NEU).is_empty());
    state
}

#[test]
fn code_renewal_task_offline_contact_connects_with_the_old_data_and_loses_no_access() {
    let scene = Scene::new(221);
    let state = running_out_for_p_and_q(&scene);
    // Without a process view the old lookup id is unknown to the host.
    assert_eq!(
        scene.hello(&scene.p, ALT, ALT_SECRET),
        Err(Some(SessionDenial::CodeChanged))
    );
    scene.install(&state);
    // The contact that was offline holds the old data and a grant.
    assert_eq!(scene.hello(&scene.p, ALT, ALT_SECRET), Ok(()));
    // An interrupted renewal: the new secret under the old lookup id heals.
    assert_eq!(scene.hello(&scene.p, ALT, NEU_SECRET), Ok(()));
    // A wrong secret is still rejected as an authentication failure.
    assert_eq!(
        scene.hello(&scene.p, ALT, 99),
        Err(Some(SessionDenial::Authentication))
    );
    // Another lookup id than the current and the running out one: stale code.
    assert_eq!(
        scene.hello(&scene.p, "cr-task-fall3-unbekannt", ALT_SECRET),
        Err(Some(SessionDenial::CodeChanged))
    );
    // The current data keep working next to the old ones.
    assert_eq!(scene.hello(&scene.p, NEU, NEU_SECRET), Ok(()));
    // Revoking the right acts at once, also for the old data.
    scene.auth.lock().unwrap().direct_grants[0].state = DirectGrantState::Ignored;
    assert_eq!(
        scene.hello(&scene.p, ALT, ALT_SECRET),
        Err(Some(SessionDenial::Blocked))
    );
    scene.auth.lock().unwrap().direct_grants[0].state = DirectGrantState::Reconfirm;
    assert_eq!(
        scene.hello(&scene.p, ALT, ALT_SECRET),
        Err(Some(SessionDenial::Reconfirm))
    );
    scene.auth.lock().unwrap().direct_grants.remove(0);
    assert_eq!(scene.hello(&scene.p, ALT, ALT_SECRET), Err(None));
    // The other holder is not affected.
    assert_eq!(scene.hello(&scene.q, ALT, ALT_SECRET), Ok(()));
}

#[test]
fn code_renewal_task_last_confirmation_ends_the_old_data() {
    let scene = Scene::new(225);
    let mut state = running_out_for_p_and_q(&scene);
    scene.install(&state);
    let (p, q) = (&scene.p, &scene.q);
    assert!(registry::is_pending(
        &scene.host.node_id,
        &p.device_id,
        &p.public_key
    ));
    // p connected with the current data (exchange completed): it leaves the
    // entry, q keeps it open.
    let retired = state.confirm(&p.device_id, &p.public_key, &p.node_id, NEU, NEU);
    assert!(retired.is_empty());
    scene.install(&state);
    assert!(!registry::is_pending(
        &scene.host.node_id,
        &p.device_id,
        &p.public_key
    ));
    assert!(registry::is_pending(
        &scene.host.node_id,
        &q.device_id,
        &q.public_key
    ));
    assert_eq!(scene.hello(q, ALT, ALT_SECRET), Ok(()));
    // The last holder confirms: the old lookup id ends.
    let retired = state.confirm(&q.device_id, &q.public_key, &q.node_id, NEU, NEU);
    assert_eq!(retired, [ALT]);
    assert!(state.retiring.is_empty());
    assert!(!state.status(false, None).active);
    scene.install(&state);
    assert!(registry::retiring_secret(&scene.host.node_id, ALT).is_none());
    assert!(registry::retiring_lookups(&scene.host.node_id).is_empty());
    // The old data are a stale code now, the new ones are accepted.
    assert_eq!(
        scene.hello(p, ALT, ALT_SECRET),
        Err(Some(SessionDenial::CodeChanged))
    );
    assert_eq!(
        scene.hello(q, ALT, ALT_SECRET),
        Err(Some(SessionDenial::CodeChanged))
    );
    assert_eq!(scene.hello(p, NEU, NEU_SECRET), Ok(()));
    assert_eq!(scene.hello(q, NEU, NEU_SECRET), Ok(()));
}

#[test]
fn code_renewal_task_removed_device_gets_no_session_with_the_old_data() {
    let scene = Scene::new(229);
    let state = running_out_for_p_and_q(&scene);
    scene.install(&state);
    // x holds the old code and has a valid proof for it, but no grant (the
    // user removed it): rejected, and without a class it could learn.
    assert_eq!(scene.hello(&scene.x, ALT, ALT_SECRET), Err(None));
    // Also with the secret of the current data under the old lookup id.
    assert_eq!(scene.hello(&scene.x, ALT, NEU_SECRET), Err(None));
    // A device with the current code and no grant is classified as before.
    assert_eq!(
        scene.hello(&scene.x, NEU, NEU_SECRET),
        Err(Some(SessionDenial::NotGranted))
    );
    // Even a pending entry of x would not open a session: the grant decides.
    let mut with_x = state.clone();
    with_x.retiring[0].pending.push(renewal_peer(&scene.x));
    scene.install(&with_x);
    assert_eq!(scene.hello(&scene.x, ALT, ALT_SECRET), Err(None));
    // The bookkeeping drops a pending device without a grant.
    let mut pruned = with_x;
    assert!(pruned.prune(|peer| peer.device_id != "device-x").is_empty());
    assert_eq!(pruned.retiring[0].pending.len(), 2);
}

#[test]
fn code_renewal_task_withdrawn_keys_name_exactly_the_removed_devices() {
    let scene = Scene::new(233);
    let withdrawn = vec![scene.x.node_id.clone(), scene.x.public_key.clone()];
    registry::install(
        &scene.host.node_id,
        NEU,
        Vec::new(),
        withdrawn,
        Default::default(),
    );
    assert!(registry::is_withdrawn(&scene.x.node_id));
    assert!(registry::is_withdrawn(&scene.x.public_key));
    assert!(!registry::is_withdrawn(&scene.p.node_id));
    assert!(!registry::is_withdrawn(&scene.q.public_key));
    assert!(!registry::is_withdrawn(""));
    // A new installation replaces the list: a readmitted device is no longer
    // withdrawn.
    registry::install(
        &scene.host.node_id,
        NEU,
        Vec::new(),
        Vec::new(),
        Default::default(),
    );
    assert!(!registry::is_withdrawn(&scene.x.node_id));
}

#[test]
fn code_renewal_task_publication_follows_the_end_of_a_retiring_lookup_id() {
    let scene = Scene::new(237);
    let mut state = running_out_for_p_and_q(&scene);
    scene.install(&state);
    let node = scene.host.node_id.clone();
    // The first round announces the retiring lookup id; nothing is gone.
    assert!(registry::sync_published(&node, &[ALT.to_string()]).is_empty());
    assert!(registry::sync_published(&node, &[ALT.to_string()]).is_empty());
    assert_eq!(registry::retiring_lookups(&node).len(), 1);
    // The last holders confirm: the old lookup id ends, the next round
    // announces nothing and gets the id to unpublish exactly once.
    for device in [&scene.p, &scene.q] {
        state.confirm(
            &device.device_id,
            &device.public_key,
            &device.node_id,
            NEU,
            NEU,
        );
    }
    scene.install(&state);
    assert!(registry::retiring_lookups(&node).is_empty());
    assert_eq!(registry::sync_published(&node, &[]), [ALT]);
    assert!(registry::sync_published(&node, &[]).is_empty());
    // A node without a view has nothing to unpublish.
    assert!(registry::sync_published("cr-task-fall3-no-view", &[]).is_empty());
}

fn legacy_presence(client: &ShareIdentity, lookup_id: &str, secret: u8) -> PeerPresence {
    let mut presence = PeerPresence {
        kind: "direct".into(),
        relation_id: lookup_id.into(),
        device_id: client.device_id.clone(),
        device_name: client.device_name.clone(),
        public_key: client.public_key.clone(),
        fingerprint: client.fingerprint.clone(),
        node_id: client.node_id.clone(),
        relay_url: "https://relay.invalid".into(),
        candidates: vec!["127.0.0.1:9000".into()],
        expires_at: 200,
        nonce: format!("nonce-{lookup_id}"),
        proof: String::new(),
    };
    let payload = presence_payload(
        "direct",
        lookup_id,
        &presence.device_id,
        &presence.public_key,
        &presence.node_id,
        &presence.relay_url,
        &presence.candidates,
        presence.expires_at,
        &presence.nonce,
    );
    presence.proof = hmac_proof(&[secret; 32], &payload);
    presence
}

#[test]
fn code_renewal_task_old_requests_stay_valid_while_their_lookup_id_runs_out() {
    let scene = Scene::new(241);
    let state = running_out_for_p_and_q(&scene);
    scene.install(&state);
    let gone = "cr-task-fall3-gone";
    let mut profiles = ShareProfiles::default();
    profiles
        .direct_grants
        .push(grant(&scene.p, DirectGrantState::Accepted));
    profiles
        .direct_grants
        .push(grant(&scene.q, DirectGrantState::Accepted));
    let policy = DirectRequestPolicy::Ask;
    // p asked under the lookup id that runs out, q under one that ended
    // without a renewal (an ordinary stale request).
    let from_p = legacy_presence(&scene.p, ALT, ALT_SECRET);
    let from_q = legacy_presence(&scene.q, gone, 77);
    assert!(profiles
        .record_verified_legacy_direct_request(ALT, &from_p, 100, policy)
        .unwrap());
    assert!(profiles
        .record_verified_legacy_direct_request(gone, &from_q, 100, policy)
        .unwrap());
    // The stored proof of the running out lookup id verifies with its secret
    // as long as the process view lists it; the current identity alone would
    // refuse it.
    let host = &scene.host;
    let entry_p = profiles
        .legacy_direct_requests
        .iter()
        .find(|entry| entry.lookup_id == ALT)
        .unwrap();
    assert!(entry_p.verify_evidence(host).is_ok());
    assert!(entry_p.verify_for_local_identity(host, 150).is_err());
    let entry_q = profiles
        .legacy_direct_requests
        .iter()
        .find(|entry| entry.lookup_id == gone)
        .unwrap();
    assert!(entry_q.verify_evidence(host).is_err());
    // Reconciling against the current lookup id keeps the entry and the grant
    // of p unchanged and still suspends the grant behind the stale one.
    assert_eq!(profiles.reconcile_legacy_identity(NEU, 300).unwrap(), 1);
    let grant_state = |profiles: &ShareProfiles, device: &str| {
        profiles.grant_for(device).map(|grant| grant.state.clone())
    };
    assert_eq!(
        grant_state(&profiles, "device-p"),
        Some(DirectGrantState::Accepted)
    );
    assert_eq!(
        grant_state(&profiles, "device-q"),
        Some(DirectGrantState::Reconfirm)
    );
    assert_eq!(profiles.legacy_direct_requests.len(), 1);
    assert_eq!(profiles.legacy_direct_requests[0].lookup_id, ALT);
    // When the old lookup id ends, its requests are forgotten and no grant
    // changes (every holder already uses the current data).
    assert!(profiles.names_direct_lookup(ALT));
    assert_eq!(profiles.forget_retired_direct_lookup(ALT), 1);
    assert!(!profiles.names_direct_lookup(ALT));
    assert_eq!(
        grant_state(&profiles, "device-p"),
        Some(DirectGrantState::Accepted)
    );
}
