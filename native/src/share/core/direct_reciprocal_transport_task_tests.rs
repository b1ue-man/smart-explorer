use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Semaphore;

use super::{
    admit_incoming_direct_repair, direct_repair_runtime_guard, persist_receiver,
    publish_runtime_profiles_committed, shared_direct_repair_store,
};
use crate::share::core::{hmac_proof, public_fingerprint};
use crate::share::direct_protocol::DirectPeerIdentity;
use crate::share::direct_reciprocal::DirectRelationMaterial;
use crate::share::direct_reciprocal_session::{
    AuthenticatedDirectSession, DirectRepairInitiator, DirectRepairReceiver,
    DirectSessionAuthorization,
};
use crate::share::direct_reciprocal_store::{
    DirectRepairPersistRequest, DirectRepairStore, DirectRepairStoreError,
    DirectRepairStoreReceipt,
};
use crate::share::direct_reciprocal_wire::{
    DirectRepairPersisted, DIRECT_RECIPROCAL_CAPABILITY,
};
use crate::share::fs::ShareExportConfig;
use crate::share::identity::ShareIdentity;
use crate::share::session::{authenticate_incoming_session, session_payload};
use crate::share::types::{
    DirectGrant, DirectGrantState, ShareAuthState, ShareEvent,
};
use crate::share::wire::PeerHello;

#[test]
fn share_remote_task_reciprocal_incoming_auth_is_reread_after_transition_permit() {
    let runtime = runtime();
    runtime.block_on(async {
        let (session, auth) = incoming_session_fixture();
        let incoming_slots = Arc::new(Semaphore::new(1));
        let transition_slots = Arc::new(Semaphore::new(1));
        let held_transition = transition_slots.clone().acquire_owned().await.unwrap();
        let admission = tokio::spawn(admit_incoming_direct_repair(
            tokio::time::Instant::now() + Duration::from_secs(2),
            session,
            auth.clone(),
            incoming_slots.clone(),
            transition_slots.clone(),
        ));

        tokio::time::timeout(Duration::from_secs(1), async {
            while incoming_slots.available_permits() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("incoming admission never reached the transition gate");
        auth.lock().unwrap().direct_grants[0].state = DirectGrantState::Ignored;
        drop(held_transition);

        let result = admission.await.unwrap();
        assert!(result.is_err(), "revoked authorization was cached before the permit");
        assert_eq!(incoming_slots.available_permits(), 1);
        assert_eq!(transition_slots.available_permits(), 1);
    });
}

#[test]
fn share_remote_task_reciprocal_timeout_holds_transition_and_incoming_slots_until_store_finishes()
{
    let runtime = runtime();
    runtime.block_on(async {
        let state = receiver_awaiting_store();
        let (entered_tx, entered_rx) = sync_channel(0);
        let (release_tx, release_rx) = sync_channel(0);
        let (finished_tx, finished_rx) = sync_channel(0);
        let store = shared_direct_repair_store(BlockingStore {
            entered: entered_tx,
            release: release_rx,
            finished: finished_tx,
        });
        let transition_slots = Arc::new(Semaphore::new(1));
        let incoming_slots = Arc::new(Semaphore::new(1));
        let guard = direct_repair_runtime_guard(
            transition_slots.clone().try_acquire_owned().unwrap(),
            Some(incoming_slots.clone().try_acquire_owned().unwrap()),
        );
        let mut pending = tokio::spawn(persist_receiver(state, store, guard));

        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("blocking store was not entered");
        assert_eq!(transition_slots.available_permits(), 0);
        assert_eq!(incoming_slots.available_permits(), 0);
        assert!(tokio::time::timeout(Duration::from_millis(25), &mut pending)
            .await
            .is_err());
        pending.abort();
        assert_eq!(transition_slots.available_permits(), 0);
        assert_eq!(incoming_slots.available_permits(), 0);

        release_tx.send(()).unwrap();
        finished_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("blocking store did not finish");
        let _transition = tokio::time::timeout(
            Duration::from_secs(1),
            transition_slots.clone().acquire_owned(),
        )
        .await
        .expect("transition permit remained held after the store finished")
        .unwrap();
        let _incoming = tokio::time::timeout(
            Duration::from_secs(1),
            incoming_slots.clone().acquire_owned(),
        )
        .await
        .expect("incoming permit remained held after the store finished")
        .unwrap();
    });
}

#[test]
fn share_remote_task_reciprocal_full_event_channel_does_not_block_tokio_worker() {
    let runtime = runtime();
    let (events, receiver) = crossbeam_channel::bounded(1);
    events.send(ShareEvent::Status("occupied".into())).unwrap();
    runtime.block_on(async {
        let completed = tokio::time::timeout(Duration::from_millis(250), tokio::spawn(async move {
            publish_runtime_profiles_committed(&events)
        }))
        .await
        .expect("a full ShareEvent channel blocked the Tokio worker")
        .expect("event worker panicked");
        assert!(completed.is_ok());
    });
    assert!(matches!(receiver.recv().unwrap(), ShareEvent::Status(_)));
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

fn incoming_session_fixture() -> (
    Arc<crate::share::session::IncomingSession>,
    Arc<Mutex<ShareAuthState>>,
) {
    let local = share_identity(31, "local", "local-lookup", 41);
    let remote_key = iroh::SecretKey::from_bytes(&[32; 32]);
    let remote = DirectPeerIdentity::from_secret("remote", "Remote", &remote_key);
    let auth = Arc::new(Mutex::new(ShareAuthState {
        identity: local.clone(),
        direct_secret: local.direct_secret(),
        default_direct_exports: ShareExportConfig::default(),
        direct_contacts: Vec::new(),
        direct_grants: vec![direct_grant(&remote)],
        rooms: Vec::new(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online: true,
        authorization_epoch: 7,
    }));
    let nonce = "admission-nonce";
    let payload = session_payload(
        "direct",
        &local.direct_lookup_id,
        &remote.device_id,
        &local.device_id,
        &remote.node_id,
        &local.node_id,
        nonce,
    );
    let hello = PeerHello {
        protocol_version: 3,
        relation_kind: "direct".into(),
        relation_id: local.direct_lookup_id.clone(),
        device_id: remote.device_id.clone(),
        public_key: remote.public_key.clone(),
        node_id: remote.node_id.clone(),
        session_nonce: nonce.into(),
        session_proof: hmac_proof(&local.direct_secret(), &payload),
        requested_capabilities: vec![DIRECT_RECIPROCAL_CAPABILITY.into()],
    };
    let session = authenticate_incoming_session(&hello, &remote.node_id, &auth).unwrap();
    (Arc::new(session), auth)
}

fn receiver_awaiting_store(
) -> crate::share::direct_reciprocal_session::DirectRepairReceiverAwaitingStore {
    let local_key = iroh::SecretKey::from_bytes(&[51; 32]);
    let remote_key = iroh::SecretKey::from_bytes(&[52; 32]);
    let local = DirectPeerIdentity::from_secret("local", "Local", &local_key);
    let remote = DirectPeerIdentity::from_secret("remote", "Remote", &remote_key);
    let local_material = DirectRelationMaterial::new("local-lookup", vec![61; 32]).unwrap();
    let remote_material = DirectRelationMaterial::new("remote-lookup", vec![62; 32]).unwrap();
    let outgoing = authenticated_session(
        &remote,
        DirectSessionAuthorization::OutgoingAcceptedContact,
    );
    let incoming = authenticated_session(
        &local,
        DirectSessionAuthorization::IncomingAcceptedGrant,
    );
    let (_, hello) = DirectRepairInitiator::begin(
        local.clone(),
        &local_material,
        outgoing,
        Some(remote_material.clone()),
    )
    .unwrap();
    DirectRepairReceiver::new(
        remote,
        remote_material,
        incoming,
        Some(local_material),
    )
    .unwrap()
    .accept_hello(hello)
    .unwrap()
}

fn authenticated_session(
    remote: &DirectPeerIdentity,
    authorization: DirectSessionAuthorization,
) -> AuthenticatedDirectSession {
    AuthenticatedDirectSession::from_verified_handshake(
        remote.device_id.clone(),
        remote.node_id.clone(),
        remote.public_key.clone(),
        remote.fingerprint.clone(),
        remote.node_id.clone(),
        authorization,
        true,
    )
    .unwrap()
}

fn share_identity(seed: u8, device_id: &str, lookup_id: &str, secret: u8) -> ShareIdentity {
    let key = iroh::SecretKey::from_bytes(&[seed; 32]);
    let public_key = key.public().to_string();
    ShareIdentity {
        device_id: device_id.into(),
        device_name: device_id.into(),
        direct_lookup_id: lookup_id.into(),
        public_key: public_key.clone(),
        fingerprint: public_fingerprint(public_key.as_bytes()),
        node_id: public_key,
        iroh_secret: key,
        direct_secret: [secret; 32],
    }
}

fn direct_grant(identity: &DirectPeerIdentity) -> DirectGrant {
    DirectGrant {
        device_id: identity.device_id.clone(),
        device_name: identity.device_name.clone(),
        public_key: identity.public_key.clone(),
        fingerprint: identity.fingerprint.clone(),
        node_id: identity.node_id.clone(),
        state: DirectGrantState::Accepted,
        updated_at: 1,
        exec: Default::default(),
    }
}

struct BlockingStore {
    entered: SyncSender<()>,
    release: Receiver<()>,
    finished: SyncSender<()>,
}

impl DirectRepairStore for BlockingStore {
    fn persist_reciprocal_peer(
        &mut self,
        request: &DirectRepairPersistRequest<'_>,
    ) -> Result<DirectRepairStoreReceipt, DirectRepairStoreError> {
        self.entered.send(()).unwrap();
        self.release.recv().unwrap();
        let receipt = request.receipt_after_durable_commit(DirectRepairPersisted::Changed);
        self.finished.send(()).unwrap();
        Ok(receipt)
    }
}
