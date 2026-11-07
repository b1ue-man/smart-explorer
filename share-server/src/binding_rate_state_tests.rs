//! New key bindings per source reach publication and registration
//! (R-SERVER-02): an empty bucket refuses only what would bind something new.

use std::collections::HashSet;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh_base::SecretKey;

use crate::limits::SourceKey;
use crate::state::{register_client_with_identity, ClientIdentity, RegistrationError, State};
use crate::{direct_presence, login, Out, PeerPresence, Writer};

/// The burst of new bindings one source may make at once.
const BURST: usize = 512;

struct Client {
    id: u64,
    device: String,
    key: SecretKey,
    writer: Writer,
    receiver: Receiver<Out>,
}

fn register(
    state: &Arc<Mutex<State>>,
    source: SourceKey,
    device: &str,
    seed: u8,
) -> Result<Client, RegistrationError> {
    let key = SecretKey::from_bytes(&[seed; 32]);
    let (writer, receiver) = Writer::test_channel(32);
    register_client_with_identity(
        state,
        writer.clone(),
        source,
        device.into(),
        HashSet::new(),
        ClientIdentity::Proven(key.public()),
    )
    .map(|id| Client {
        id,
        device: device.into(),
        key,
        writer,
        receiver,
    })
}

fn presence(client: &Client, lookup: &str) -> PeerPresence {
    PeerPresence {
        kind: "direct".into(),
        relation_id: lookup.into(),
        device_id: client.device.clone(),
        device_name: client.device.clone(),
        public_key: client.key.public().to_string(),
        fingerprint: "fp".into(),
        node_id: client.key.public().to_string(),
        relay_url: "https://relay.example".into(),
        candidates: Vec::new(),
        expires_at: crate::discovery_state::unix_seconds() + 300,
        nonce: "nonce".into(),
        proof: "proof".into(),
    }
}

fn publish(state: &Arc<Mutex<State>>, client: &Client, lookup: &str) {
    direct_presence::publish(
        client.id,
        &client.writer,
        presence(client, lookup),
        Some(login::hex(&login::sha256(&[5; 32]))),
        state,
    );
}

fn published(state: &Arc<Mutex<State>>, lookup: &str) -> bool {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .direct
        .contains_key(lookup)
}

#[test]
fn an_empty_binding_bucket_refuses_only_new_bindings_of_that_source() {
    let state = Arc::new(Mutex::new(State::default()));
    let noisy_source = SourceKey::Ipv4([192, 0, 2, 9]);
    let quiet_source = SourceKey::Ipv4([192, 0, 2, 10]);
    // The registration of a new device id takes the first token.
    let noisy = register(&state, noisy_source, "noisy", 91).expect("first device");
    for index in 0..BURST - 1 {
        let lookup = format!("lookup-{index}");
        publish(&state, &noisy, &lookup);
        assert!(published(&state, &lookup), "binding {index} is accepted");
        // One client holds at most 64 Direct lookups at a time.
        direct_presence::unpublish(noisy.id, &lookup, &state);
    }

    // The bucket is empty: one more new lookup is refused with its reason.
    publish(&state, &noisy, "lookup-over");
    assert!(!published(&state, "lookup-over"));
    match noisy.receiver.recv_timeout(Duration::from_secs(2)) {
        Ok(Out::Error { scope, msg }) => {
            assert_eq!(scope, "direct");
            assert_eq!(
                msg,
                "too many new key bindings from this network; retry later"
            );
        }
        _ => panic!("the publication was not refused with an error"),
    }
    assert!(state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .bindings
        .is_new_lookup("lookup-over"));

    // A lookup the key already owns needs no token.
    publish(&state, &noisy, "lookup-0");
    assert!(published(&state, "lookup-0"));
    // Another source publishes as before.
    let quiet = register(&state, quiet_source, "quiet", 92).expect("other source");
    publish(&state, &quiet, "quiet-lookup");
    assert!(published(&state, "quiet-lookup"));

    // A registration with a new device id is refused for the empty source ...
    assert_eq!(
        register(&state, noisy_source, "second-device", 93).err(),
        Some(RegistrationError::BindingRateLimited)
    );
    assert_eq!(
        RegistrationError::BindingRateLimited.message(),
        "too many new device bindings from this network; retry later"
    );
    // ... but not with a device id that is already bound to the key.
    assert!(register(&state, noisy_source, "noisy", 91).is_ok());
    assert!(register(&state, quiet_source, "second-device", 94).is_ok());
}
