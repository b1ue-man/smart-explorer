use super::*;
use crate::vfs::{Backend, Scheme, VfsMeta};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};

struct InterruptingPeer {
    fixture: Arc<CopyPastePeerFixture>,
    peer: Arc<PeerBackend>,
    opens: AtomicUsize,
    revoke: bool,
}

struct InterruptedRead {
    reader: Box<dyn Read + Send>,
    fixture: Arc<CopyPastePeerFixture>,
    peer: Arc<PeerBackend>,
    interrupted: bool,
    revoke: bool,
}

impl Read for InterruptedRead {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() { return Ok(0); }
        if self.interrupted {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "injected physical disconnect"));
        }
        let n = self.reader.read(out)?;
        if n > 0 {
            self.peer.node.disconnect_outgoing_for_test(self.peer.initial_endpoint())?;
            if self.revoke { self.fixture.revoke_access()?; }
            self.interrupted = true;
        }
        Ok(n)
    }
}

impl Backend for InterruptingPeer {
    fn scheme(&self) -> Scheme { Scheme::Peer }
    fn root_display(&self) -> String { "/".into() }
    fn list_dir(&self, path: &str) -> io::Result<Vec<VfsMeta>> { self.peer.list_dir(path) }
    fn stat(&self, path: &str) -> io::Result<VfsMeta> { self.peer.stat(path) }
    fn open_read(&self, path: &str) -> io::Result<Box<dyn Read + Send>> {
        let reader = self.peer.open_read(path)?;
        if self.opens.fetch_add(1, Ordering::SeqCst) == 0 {
            Ok(Box::new(InterruptedRead { reader, fixture: self.fixture.clone(),
                peer: self.peer.clone(), interrupted: false, revoke: self.revoke }))
        } else { Ok(reader) }
    }
    fn open_write(&self, _: &str) -> io::Result<Box<dyn Write + Send>> { panic!("read mutated peer") }
    fn rename(&self, _: &str, _: &str) -> io::Result<()> { panic!("read renamed peer") }
    fn remove_file(&self, _: &str) -> io::Result<()> { panic!("read deleted peer") }
    fn remove_dir(&self, _: &str) -> io::Result<()> { panic!("read deleted peer") }
    fn mkdir_all(&self, _: &str) -> io::Result<()> { panic!("read created directory") }
}

fn room_peer(fixture: &CopyPastePeerFixture) -> (Arc<PeerBackend>, String) {
    use crate::share::{RoomMember, ShareProfiles};
    let material = crate::share::room_relation::RoomRelationMaterial::new(
        "direct-open-task-room", vec![71; 32]).unwrap();
    let (profiles, outcome) = ShareProfiles::add_room_material_persisted(
        None, &material, "Open task room").unwrap();
    let id = outcome.room_profile_id().to_string();
    let mut room = profiles.rooms.into_iter().find(|r| r.id == id).unwrap();
    let client = &fixture.peer.identity;
    room.exports = fixture.host_auth.lock().unwrap().default_direct_exports.clone();
    room.members.push(RoomMember {
        device_id: client.device_id.clone(), device_name: client.device_name.clone(),
        fingerprint: client.fingerprint.clone(), public_key: client.public_key.clone(),
        node_id: client.node_id.clone(), relay_url: String::new(), candidates: Vec::new(),
        last_seen: None, status: ShareStatus::Available, blocked: false,
        exec: Default::default(), presence: None,
    });
    fixture.host_auth.lock().unwrap().rooms.push(room);
    let mut endpoint = fixture.peer.initial_endpoint().clone();
    endpoint.scope = ShareScope::Room { room_id: material.room_id().into() };
    endpoint.presence.kind = "room".into();
    endpoint.presence.relation_id = material.room_id().into();
    endpoint.relation_secret = material.secret().to_vec();
    (Arc::new(PeerBackend::new(endpoint, client.clone(), fixture.peer.node.clone())), id)
}

#[test]
fn direct_open_task_real_direct_and_room_reconnect_through_daemon_without_replay() {
    for room in [false, true] {
        let fixture = Arc::new(CopyPastePeerFixture::new().unwrap());
        let (peer, room_id) = if room {
            let (peer, id) = room_peer(&fixture);
            (peer, Some(id))
        } else { (fixture.peer.clone(), None) };
        let contents: Vec<u8> = (0..256 * 1024).map(|i| (i % 251) as u8).collect();
        std::fs::write(fixture.root_a.join("photo Ü #.jpg"), &contents).unwrap();
        peer.stat("/A/photo Ü #.jpg").unwrap();
        let before = peer.node.outgoing_generation_for_test(peer.initial_endpoint()).unwrap();
        let source = Arc::new(InterruptingPeer { fixture: fixture.clone(), peer: peer.clone(),
            opens: AtomicUsize::new(0), revoke: false });
        let bridge = crate::daemon::DirectOpenTaskBridge::new(source.clone()).unwrap();
        let cached = crate::vfs::CachingBackend::new(bridge.backend.clone());
        cached.list_dir("/A").unwrap();
        let dest = tempfile::tempdir().unwrap();
        let path = dest.path().join("photo Ü #.jpg");
        crate::transfer::download_for_edit(&cached, "/A/photo Ü #.jpg", None, &path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), contents);
        assert_eq!(source.opens.load(Ordering::SeqCst), 2);
        assert_eq!(std::fs::read_dir(dest.path()).unwrap().count(), 1);
        assert_ne!(peer.node.outgoing_generation_for_test(peer.initial_endpoint()).unwrap(), before);
        assert_eq!(std::fs::read(fixture.root_a.join("photo Ü #.jpg")).unwrap(), contents);
        drop(bridge);
        if let Some(id) = room_id {
            crate::share::ShareProfiles::remove_room_persisted(None, &id).unwrap();
        }
    }
}

#[test]
fn direct_open_task_reconnect_cannot_bypass_revoked_direct_access() {
    let fixture = Arc::new(CopyPastePeerFixture::new().unwrap());
    std::fs::write(fixture.root_a.join("file"), vec![1; 256 * 1024]).unwrap();
    let source = Arc::new(InterruptingPeer { peer: fixture.peer.clone(), fixture,
        opens: AtomicUsize::new(0), revoke: true });
    let bridge = crate::daemon::DirectOpenTaskBridge::new(source.clone()).unwrap();
    let dest = tempfile::tempdir().unwrap();
    assert!(crate::transfer::download_for_edit(&*bridge.backend, "/A/file", None,
        &dest.path().join("file")).is_err());
    assert_eq!(source.opens.load(Ordering::SeqCst), 1);
    assert_eq!(std::fs::read_dir(dest.path()).unwrap().count(), 0);
}
