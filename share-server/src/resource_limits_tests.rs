use std::collections::HashSet;
use std::io::{ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tungstenite::Message;

use super::limits::{
    SourceKey, MAX_PUBLISHED_DIRECTS_PER_CLIENT, MAX_REGISTERED_CLIENTS,
    MAX_REGISTERED_CLIENTS_PER_SOURCE, MAX_ROOMS_PER_CLIENT, MAX_WATCHES_PER_CLIENT,
};
use super::state::{
    cleanup, lock_state, register_client, register_client_with_identity, ClientIdentity,
    RegistrationError, State,
};
use super::transport::{handle, handle_with_timeout};
use super::{direct_presence, login, rooms, tracked_direct, Out, PeerPresence, Writer};

#[test]
fn registered_clients_are_hard_capped_and_all_slots_are_reusable() {
    let state = Arc::new(Mutex::new(State::default()));
    let mut receivers = Vec::new();
    let mut ids = Vec::new();
    for index in 0..MAX_REGISTERED_CLIENTS {
        let (writer, receiver) = Writer::test_raw_channel(1);
        let id = register_client(
            &state,
            writer,
            source(index),
            format!("device-{index}"),
            HashSet::new(),
        )
        .unwrap();
        receivers.push(receiver);
        ids.push(id);
    }

    let (overflow, _receiver) = Writer::test_raw_channel(1);
    assert_eq!(
        register_client(
            &state,
            overflow,
            source(MAX_REGISTERED_CLIENTS),
            "overflow".into(),
            HashSet::new()
        ),
        Err(RegistrationError::Full)
    );
    assert_eq!(lock_state(&state).clients.len(), MAX_REGISTERED_CLIENTS);

    for id in ids {
        cleanup(id, &state);
    }
    assert!(lock_state(&state).clients.is_empty());

    let (replacement, _receiver) = Writer::test_raw_channel(1);
    assert!(register_client(
        &state,
        replacement,
        source(0),
        "replacement".into(),
        HashSet::new()
    )
    .is_ok());
    drop(receivers);
}

#[test]
fn registered_client_source_cap_preserves_slots_for_other_sources() {
    let state = Arc::new(Mutex::new(State::default()));
    let same_source = source(1);
    let mut receivers = Vec::new();
    let mut ids = Vec::new();
    for index in 0..MAX_REGISTERED_CLIENTS_PER_SOURCE {
        let (writer, receiver) = Writer::test_raw_channel(1);
        ids.push(
            register_client(
                &state,
                writer,
                same_source,
                format!("same-source-{index}"),
                HashSet::new(),
            )
            .unwrap(),
        );
        receivers.push(receiver);
    }
    let (overflow, _receiver) = Writer::test_raw_channel(1);
    assert_eq!(
        register_client(
            &state,
            overflow,
            same_source,
            "same-source-overflow".into(),
            HashSet::new(),
        ),
        Err(RegistrationError::SourceFull)
    );

    let (other, other_receiver) = Writer::test_raw_channel(1);
    let other_id = register_client(
        &state,
        other,
        source(2),
        "other-source".into(),
        HashSet::new(),
    )
    .unwrap();
    assert_eq!(lock_state(&state).clients.len(), ids.len() + 1);

    for id in ids.into_iter().chain(std::iter::once(other_id)) {
        cleanup(id, &state);
    }
    drop((receivers, other_receiver));
    assert!(lock_state(&state).clients.is_empty());
}

#[test]
fn retained_state_caps_reject_growth_and_cleanup_returns_to_baseline() {
    let state = Arc::new(Mutex::new(State::default()));
    let (writer, _receiver) = Writer::test_raw_channel(512);
    // E60: a client without key login publishes no Direct presence, so the client logs in
    // with its key; its presences carry that key and every relation shows its proof.
    let key = iroh_base::SecretKey::from_bytes(&[90; 32]);
    let proof = login::hex(&[5; 32]);
    let hash = login::hex(&login::sha256(&[5; 32]));
    let own = |relation: &str, kind: &str| {
        let mut own = presence("device", relation);
        own.kind = kind.into();
        own.public_key = key.public().to_string();
        own.node_id = key.public().to_string();
        own
    };
    let id = register_client_with_identity(
        &state,
        writer.clone(),
        source(0),
        "device".into(),
        HashSet::new(),
        ClientIdentity::Proven(key.public()),
    )
    .unwrap();

    for index in 0..MAX_PUBLISHED_DIRECTS_PER_CLIENT {
        let presence = own(&format!("direct-{index}"), "direct");
        direct_presence::publish(id, &writer, presence, Some(hash.clone()), &state);
    }
    direct_presence::publish(
        id,
        &writer,
        own("direct-overflow", "direct"),
        Some(hash.clone()),
        &state,
    );

    for index in 0..MAX_WATCHES_PER_CLIENT {
        let lookup = format!("watch-{index}");
        direct_presence::watch(id, &writer, &lookup, Some(proof.clone()), &state);
    }
    direct_presence::watch(id, &writer, "watch-overflow", Some(proof.clone()), &state);

    for index in 0..MAX_ROOMS_PER_CLIENT {
        let room_id = format!("room-{index}");
        let room_presence = own(&room_id, "room");
        rooms::join_with_access(
            id,
            &writer,
            &room_id,
            room_presence,
            Some(proof.clone()),
            &state,
        );
    }
    rooms::join_with_access(
        id,
        &writer,
        "room-overflow",
        own("room-overflow", "room"),
        Some(proof.clone()),
        &state,
    );

    let before_invalid = state_counts(&state);
    let oversized = "x".repeat(257);
    let mut invalid_presence = own("invalid-direct", "direct");
    invalid_presence.device_name = "n".repeat(1025);
    direct_presence::publish(id, &writer, invalid_presence, Some(hash), &state);
    direct_presence::watch(id, &writer, &oversized, Some(proof.clone()), &state);
    rooms::join_with_access(
        id,
        &writer,
        &oversized,
        own("invalid-room", "room"),
        Some(proof),
        &state,
    );

    assert_eq!(
        before_invalid,
        (
            1,
            MAX_PUBLISHED_DIRECTS_PER_CLIENT,
            MAX_WATCHES_PER_CLIENT,
            MAX_ROOMS_PER_CLIENT,
        )
    );
    assert_eq!(state_counts(&state), before_invalid);

    cleanup(id, &state);
    assert_eq!(state_counts(&state), (0, 0, 0, 0));
}

#[test]
fn watch_unwatch_churn_does_not_retain_empty_lookup_keys() {
    let state = Arc::new(Mutex::new(State::default()));
    let (writer, _receiver) = Writer::test_raw_channel(1);
    let id = register_client(
        &state,
        writer.clone(),
        source(0),
        "device".into(),
        HashSet::new(),
    )
    .unwrap();

    for index in 0..1_000 {
        let lookup_id = format!("lookup-{index}");
        direct_presence::watch(id, &writer, &lookup_id, None, &state);
        tracked_direct::unwatch(id, &lookup_id, &state);
    }

    let state = lock_state(&state);
    assert!(state.watchers.is_empty());
    assert!(state.clients[&id].watched_lookup_ids.is_empty());
}

#[test]
fn websocket_queue_saturation_drops_excess_without_disconnect() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let state = Arc::new(Mutex::new(State::default()));
    let server_state = state.clone();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        handle(stream, server_state)
    });

    let (mut websocket, _) = tungstenite::connect(format!("ws://{address}/se-share")).unwrap();
    websocket
        .send(Message::Text(
            r#"{"t":"hello","protocol_version":3,"device_id":"saturated","device_name":"Saturated","listen_port":0,"lan":[],"public_key":"pk","fingerprint":"fp"}"#
                .to_string(),
        ))
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&websocket.read().unwrap().into_text().unwrap())
            .unwrap()["t"],
        "hello_ok"
    );

    let writer = wait_for_registered_writer(&state);
    // The client stops reading. Large messages fill both socket buffers, the
    // writer thread blocks inside its write timeout and the bounded queue
    // (count or byte budget) then refuses further messages; tiny messages
    // alone could be drained as fast as they are queued.
    let large = Out::Error {
        scope: "saturation".into(),
        msg: "x".repeat(200 * 1024),
    };
    let mut rejected = false;
    for _ in 0..4_096 {
        if !writer.try_send(&large) {
            rejected = true;
            break;
        }
    }
    assert!(rejected, "bounded queue did not saturate");
    assert!(!writer.is_closed());

    drop(websocket);
    let _ = server.join().unwrap();
    assert!(lock_state(&state).clients.is_empty());
}

#[test]
fn tcp_hello_has_an_absolute_pre_registration_deadline() {
    // Rows: a TCP hello and a WebSocket upgrade, each drip-fed one byte at a
    // time. Both hit the absolute deadline: a timeout between 100 ms and 1 s,
    // and no registered client afterwards.
    let cases: [(&str, &[u8], u8); 2] = [
        ("tcp hello", b"{", b' '),
        (
            "websocket upgrade",
            b"GET / HTTP/1.1\r\nHost: localhost\r\nX-Slow:",
            b'a',
        ),
    ];
    for (case, prefix, fill) in cases {
        let (listener, address) = loopback_listener();
        let state = Arc::new(Mutex::new(State::default()));
        let server_state = state.clone();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect(case);
            handle_with_timeout(stream, server_state, Duration::from_millis(150))
        });

        let client = TcpStream::connect(address).expect(case);
        let dripper = std::thread::spawn(move || drip_slowloris(client, prefix, fill));
        let started = Instant::now();
        let error = server.join().expect(case).expect_err(case);
        let elapsed = started.elapsed();
        assert_eq!(error.kind(), ErrorKind::TimedOut, "{case}: {error}");
        assert!(elapsed >= Duration::from_millis(100), "{case}");
        assert!(elapsed < Duration::from_secs(1), "{case}");
        dripper.join().expect(case);
        assert!(lock_state(&state).clients.is_empty(), "{case}");
    }
}

#[test]
fn websocket_hello_shares_the_absolute_upgrade_deadline() {
    let (listener, address) = loopback_listener();
    let state = Arc::new(Mutex::new(State::default()));
    let server_state = state.clone();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        handle_with_timeout(stream, server_state, Duration::from_millis(200))
    });

    let (websocket, _) = tungstenite::connect(format!("ws://{address}/se-share")).unwrap();
    let error = server.join().unwrap().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::TimedOut, "{error}");
    drop(websocket);
    assert!(lock_state(&state).clients.is_empty());
}

#[test]
fn websocket_busy_frames_cannot_extend_pre_registration_deadline() {
    let (listener, address) = loopback_listener();
    let state = Arc::new(Mutex::new(State::default()));
    let server_state = state.clone();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        handle_with_timeout(stream, server_state, Duration::from_millis(200))
    });

    let (mut websocket, _) = tungstenite::connect(format!("ws://{address}/se-share")).unwrap();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(1) {
        if websocket.send(Message::Text("{}".into())).is_err() {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let error = server.join().unwrap().unwrap_err();
    assert_eq!(error.kind(), ErrorKind::TimedOut, "{error}");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(lock_state(&state).clients.is_empty());
}

fn wait_for_registered_writer(state: &Arc<Mutex<State>>) -> Writer {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Some(writer) = lock_state(state)
            .clients
            .values()
            .next()
            .map(|client| client.writer.clone())
        {
            return writer;
        }
        assert!(Instant::now() < deadline, "client was not registered");
        std::thread::yield_now();
    }
}

fn state_counts(state: &Arc<Mutex<State>>) -> (usize, usize, usize, usize) {
    let state = lock_state(state);
    (
        state.clients.len(),
        state.direct.len(),
        state.watchers.len(),
        state.rooms.len(),
    )
}

fn presence(device_id: &str, relation_id: &str) -> PeerPresence {
    PeerPresence {
        kind: "direct".into(),
        relation_id: relation_id.into(),
        device_id: device_id.into(),
        device_name: "Device".into(),
        public_key: "pk".into(),
        fingerprint: "fp".into(),
        node_id: "node".into(),
        relay_url: "http://127.0.0.1:51821".into(),
        candidates: vec!["127.0.0.1:1".into()],
        expires_at: 99,
        nonce: "nonce".into(),
        proof: "proof".into(),
    }
}

fn source(index: usize) -> SourceKey {
    SourceKey::Ipv4([192, 0, (index / 256) as u8, (index % 256) as u8])
}

fn loopback_listener() -> (TcpListener, std::net::SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    (listener, address)
}

fn drip_slowloris(mut stream: TcpStream, prefix: &[u8], fill: u8) {
    if stream.write_all(prefix).is_err() || stream.flush().is_err() {
        return;
    }
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(20));
        if stream.write_all(&[fill]).is_err() || stream.flush().is_err() {
            break;
        }
    }
}
