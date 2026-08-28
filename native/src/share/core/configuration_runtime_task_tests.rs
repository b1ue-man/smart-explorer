use super::schedule_snapshot_with;
use crate::share::core::{now_secs, public_fingerprint};
use crate::share::direct_reciprocal_coordinator::DirectReciprocalCoordinator;
use crate::share::fs::ShareExportConfig;
use crate::share::identity::ShareIdentity;
use crate::share::types::{
    DirectAccessState, DirectContact, PeerPresence, ShareAuthState, ShareStatus,
};

#[test]
fn share_remote_task_reciprocal_direct_offline_snapshot_plans_nothing() {
    let snapshot = eligible_snapshot(true);
    let online = DirectReciprocalCoordinator::detached_for_task_test(
        snapshot.authorization_epoch,
    );
    schedule_snapshot_with(&snapshot, &online, |_| Ok(Some(vec![71; 32])));
    assert_eq!(online.task_count_for_task_test(), 1);

    let mut offline_snapshot = snapshot;
    offline_snapshot.direct_online = false;
    let offline = DirectReciprocalCoordinator::detached_for_task_test(
        offline_snapshot.authorization_epoch,
    );
    let mut secret_reads = 0;
    schedule_snapshot_with(&offline_snapshot, &offline, |_| {
        secret_reads += 1;
        Ok(Some(vec![71; 32]))
    });
    assert_eq!(secret_reads, 0);
    assert_eq!(offline.task_count_for_task_test(), 0);
}

fn eligible_snapshot(direct_online: bool) -> ShareAuthState {
    let local = share_identity(70, "local", "local-lookup", 70);
    let remote_key = iroh::SecretKey::from_bytes(&[71; 32]);
    let remote_public = remote_key.public().to_string();
    let remote_fingerprint = public_fingerprint(remote_public.as_bytes());
    let presence = PeerPresence {
        kind: "direct".into(),
        relation_id: "remote-lookup".into(),
        device_id: "remote".into(),
        device_name: "Remote".into(),
        public_key: remote_public.clone(),
        fingerprint: remote_fingerprint.clone(),
        node_id: remote_public.clone(),
        relay_url: String::new(),
        candidates: vec!["127.0.0.1:41371".into()],
        expires_at: now_secs() + 60,
        nonce: "presence-nonce".into(),
        proof: "unused-for-planning".into(),
    };
    ShareAuthState {
        identity: local.clone(),
        direct_secret: local.direct_secret(),
        default_direct_exports: ShareExportConfig::default(),
        direct_contacts: vec![DirectContact {
            id: "remote-contact".into(),
            display_name: "Remote".into(),
            lookup_id: presence.relation_id.clone(),
            expected_fingerprint: remote_fingerprint,
            expected_node_id: remote_public.clone(),
            remote_device_id: Some(presence.device_id.clone()),
            remote_public_key: Some(remote_public.clone()),
            auto_connect: true,
            auto_open: false,
            last_seen: None,
            status: ShareStatus::Offline,
            last_error: None,
            presence: Some(presence),
            access_state: DirectAccessState::Accepted,
            request_sent_at: None,
            accepted_at: Some(now_secs()),
            accepted_public_key: Some(remote_public),
        }],
        direct_grants: Vec::new(),
        rooms: Vec::new(),
        direct_requests: Vec::new(),
        direct_request_tombstones: Vec::new(),
        seen_nonces: Default::default(),
        direct_online,
        authorization_epoch: 17,
    }
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
