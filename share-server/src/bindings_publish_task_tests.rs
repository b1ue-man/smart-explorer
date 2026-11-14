//! A direct publication reaches the disk before its entry becomes visible (R1,
//! SEC-SERVER-BESITZER-60S Teil 3): the copy with the binding is written outside the
//! state lock, a refused publication stays hidden, and a failed write keeps its
//! change pending for the next save.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh_base::SecretKey;

use crate::bindings::Bindings;
use crate::limits::SourceKey;
use crate::state::{register_client_with_identity, ClientIdentity, State};
use crate::{direct_presence, login, Out, PeerPresence, Writer};

const WAIT: Duration = Duration::from_secs(5);

/// A private directory for one state file; removed when dropped.
struct Dir {
    path: PathBuf,
}

impl Dir {
    fn new(label: &str) -> Self {
        let nonce = login::nonce().expect("random nonce");
        let path = std::env::temp_dir().join(format!(
            "se-share-publish-task-{label}-{}-{}",
            std::process::id(),
            login::hex(&nonce)
        ));
        std::fs::create_dir(&path).expect("temporary directory");
        Self { path }
    }

    fn file(&self) -> PathBuf {
        self.path.join("bindings.json")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A proven client that publishes its own device as a direct lookup.
struct Owner {
    id: u64,
    device: String,
    key: SecretKey,
    writer: Writer,
    receiver: Receiver<Out>,
}

fn new_owner(state: &Arc<Mutex<State>>, device: &str, seed: u8) -> Owner {
    let key = SecretKey::from_bytes(&[seed; 32]);
    let (writer, receiver) = Writer::test_channel(32);
    let id = register_client_with_identity(
        state,
        writer.clone(),
        SourceKey::Ipv4([192, 0, 2, seed]),
        device.into(),
        HashSet::new(),
        ClientIdentity::Proven(key.public()),
    )
    .expect("registration");
    Owner {
        id,
        device: device.into(),
        key,
        writer,
        receiver,
    }
}

fn publish(state: &Arc<Mutex<State>>, owner: &Owner, lookup: &str) {
    let presence = PeerPresence {
        kind: "direct".into(),
        relation_id: lookup.into(),
        device_id: owner.device.clone(),
        device_name: owner.device.clone(),
        public_key: owner.key.public().to_string(),
        fingerprint: "fp".into(),
        node_id: owner.key.public().to_string(),
        relay_url: "https://relay.example".into(),
        candidates: Vec::new(),
        expires_at: crate::discovery_state::unix_seconds() + 300,
        nonce: "nonce".into(),
        proof: "proof".into(),
    };
    direct_presence::publish(
        owner.id,
        &owner.writer,
        presence,
        Some(login::hex(&login::sha256(&[5; 32]))),
        state,
    );
}

#[test]
fn server_rand_task_publication_reaches_the_disk_before_it_is_visible() {
    let dir = Dir::new("visible");
    let path = dir.file();
    let mut initial = State::default();
    initial.binding_path = Some(path.clone());
    let state = Arc::new(Mutex::new(initial));
    let owner = new_owner(&state, "owner", 81);

    publish(&state, &owner, "lookup");

    // The entry is recorded, and the copy holding the binding is on disk.
    assert_eq!(state.lock().expect("state").direct["lookup"].0, owner.id);
    let saved = Bindings::load(&path, 0).expect("state file");
    let key = owner.key.public().to_string();
    assert_eq!(saved.lookup_key("lookup"), Some(key.as_str()));
    assert!(
        owner.receiver.try_recv().is_err(),
        "the owner is not refused"
    );
}

#[test]
fn server_rand_task_failed_publication_write_stays_hidden_and_pending() {
    let dir = Dir::new("failed");
    let path = dir.file();
    let state = Arc::new(Mutex::new(State::default()));
    // The owner registers without a state file; the state file is unwritable after that.
    let owner = new_owner(&state, "owner", 82);
    let missing = dir.path.join("missing").join("bindings.json");
    state.lock().expect("state").binding_path = Some(missing);

    publish(&state, &owner, "lookup");

    // The write fails: the owner is refused, and the entry stays invisible.
    assert!(matches!(
        owner.receiver.recv_timeout(WAIT),
        Ok(Out::Error { .. })
    ));
    assert!(!state.lock().expect("state").direct.contains_key("lookup"));

    // The binding stayed pending: the next publication with a working path writes it.
    state.lock().expect("state").binding_path = Some(path.clone());
    publish(&state, &owner, "lookup");
    assert_eq!(state.lock().expect("state").direct["lookup"].0, owner.id);
    let saved = Bindings::load(&path, 0).expect("state file");
    let key = owner.key.public().to_string();
    assert_eq!(saved.lookup_key("lookup"), Some(key.as_str()));
}
