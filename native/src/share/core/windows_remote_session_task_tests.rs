use super::*;
use crate::share::CopyPastePeerFixture;
use crate::vfs::Backend;
use std::time::Duration;

#[test]
fn windows_remote_task_repair_from_plain_thread_releases_transition() {
    std::thread::spawn(|| {
        let fixture = CopyPastePeerFixture::new().unwrap();
        let peer = &fixture.peer;
        let endpoint = peer.initial_endpoint().clone();
        let node = &peer.node;
        let permits = node.runtime_transition_slot.available_permits();
        let result = node.repair_direct_reciprocal(&endpoint, &peer.identity, 0);
        assert_eq!(result, DirectReciprocalTransportResult::Transient);
        assert_eq!(node.runtime_transition_slot.available_permits(), permits);
        assert_eq!(peer.list_dir("/").unwrap().len(), 2);
    }).join().expect("repair must not panic when started outside Tokio");
}

#[test]
fn windows_remote_task_handshake_queue_obeys_deadline_and_recovers() {
    let fixture = CopyPastePeerFixture::new().unwrap();
    let peer = &fixture.peer;
    let endpoint = peer.initial_endpoint().clone();
    let node = &peer.node;
    node.disconnect_outgoing_for_test(&endpoint).unwrap();
    let key = session_key(&endpoint);
    let gate = Arc::new(tokio::sync::Mutex::new(()));
    let guard = node.block_on(gate.clone().lock_owned());
    node.session_connects.lock().unwrap().insert(key, Arc::downgrade(&gate));
    let start = Instant::now();
    let result = node.open_stream_until(&endpoint, &peer.identity, start + Duration::from_millis(40));
    assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::TimedOut));
    assert!(start.elapsed() < Duration::from_secs(2));
    drop(guard);
    assert_eq!(peer.list_dir("/").unwrap().len(), 2);
}

#[test]
fn windows_remote_task_live_peer_reconnects_using_fresh_lan_evidence() {
    let fixture = CopyPastePeerFixture::new().unwrap();
    let peer = &fixture.peer;
    let mut endpoint = peer.initial_endpoint().clone();
    let crate::share::types::ShareScope::Direct { contact_id } = endpoint.scope.clone() else { panic!("Direct fixture") };
    let now = crate::share::core::now_secs();
    endpoint.presence.expires_at = now - 1;
    {
        let mut auth = peer.node.auth.lock().unwrap();
        let contact = &mut auth.direct_contacts[0];
        contact.presence = Some(endpoint.presence.clone());
        contact.lan_seen_at = Some(now);
        contact.lan_candidates = endpoint.presence.candidates.clone();
    }
    peer.node.disconnect_outgoing_for_test(&endpoint).unwrap();
    let live = crate::share::backend::PeerBackend::new_live(endpoint,
        crate::share::PeerOpenTarget::Direct { contact_id }, peer.node.auth.clone(),
        peer.identity.clone(), peer.node.clone());
    assert_eq!(live.list_dir("/").unwrap().len(), 2);
    peer.node.auth.lock().unwrap().direct_contacts[0].access_state = crate::share::DirectAccessState::Ignored;
    assert_eq!(live.list_dir("/").unwrap_err().kind(), io::ErrorKind::PermissionDenied);
}
