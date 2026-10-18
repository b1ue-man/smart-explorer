use std::sync::Arc;
use std::time::Duration;

use super::*;

fn principal(device: &str, relation_kind: &str) -> ProvenPrincipal {
    ProvenPrincipal {
        relation_kind: relation_kind.into(),
        relation_id: format!("relation-{device}"),
        device_id: device.into(),
        device_name: device.into(),
        public_key: format!("key-{device}"),
        fingerprint: format!("fp-{device}"),
        node_id: format!("key-{device}"),
    }
}

fn session(device: &str, mode: SessionMode, screen_content: bool) -> NewSession {
    NewSession {
        principal: principal(device, "direct"),
        device_name: device.to_uppercase(),
        mode,
        source: if screen_content {
            "monitor:1".into()
        } else {
            "synthetic".into()
        },
        path: "direct".into(),
        started_at: 1_760_000_000,
        state: SessionState::Starting,
        screen_content,
    }
}

/// Ends the sessions of one device that need one capability.
struct Narrowed {
    device: &'static str,
    capability: Capability,
}

impl SessionRestrictions for Narrowed {
    fn ends_session(&self, principal: &ProvenPrincipal, needs: &[Capability]) -> Option<EndReason> {
        (principal.device_id == self.device && needs.contains(&self.capability))
            .then_some(EndReason::CapabilityNarrowed)
    }
}

#[test]
fn screen_task_registry_limits_control_and_stop() {
    let registry = Arc::new(ScreenRegistry::default());
    let (revision, empty) = registry.sessions();
    assert!(empty.is_empty());

    // One device holds at most four sessions, the host at most eight.
    let mut handles = Vec::new();
    for _ in 0..MAX_DEVICE_SESSIONS {
        handles.push(
            registry
                .register(session("a", SessionMode::Preview, false))
                .unwrap(),
        );
    }
    assert_eq!(
        registry
            .register(session("a", SessionMode::View, true))
            .err(),
        Some(RegistryRefusal::SessionLimit)
    );
    for device in ["b", "c", "d", "e"] {
        handles.push(
            registry
                .register(session(device, SessionMode::View, true))
                .unwrap(),
        );
    }
    assert_eq!(
        registry
            .register(session("f", SessionMode::View, true))
            .err(),
        Some(RegistryRefusal::SessionLimit)
    );
    let (later, views) = registry.sessions();
    assert_ne!(later, revision);
    assert_eq!(views.len(), MAX_SESSIONS);
    assert_eq!(views[0].relation, "direct");
    assert_eq!(views[0].room_id, None);
    assert_eq!(registry.screen_content_sessions(), 4);

    // Control is exclusive: the second session sees the holder, and the
    // holder's release or end frees it.
    let first = registry.register(session("g", SessionMode::Control, true));
    assert_eq!(first.err(), Some(RegistryRefusal::SessionLimit));
    handles.truncate(4);
    let first = registry
        .register(session("g", SessionMode::Control, true))
        .unwrap();
    let second = registry
        .register(session("h", SessionMode::Control, true))
        .unwrap();
    first.take_control().unwrap();
    let holder = second.take_control().unwrap_err();
    assert_eq!(holder.device_id, "g");
    assert_eq!(holder.device_name, "G");
    assert!(registry.sessions().1.iter().any(|view| view.controlling));
    assert!(!second.release_control());
    assert!(first.release_control());
    second.take_control().unwrap();
    assert_eq!(first.take_control().unwrap_err().device_id, "h");
    drop(second);
    first.take_control().unwrap();
    assert_eq!(registry.controller().unwrap().device_id, "g");

    // A narrowed right ends only the sessions that need it.
    let control_signal = first.stop_signal();
    let narrowed = Narrowed {
        device: "g",
        capability: Capability::ScreenControl,
    };
    assert_eq!(registry.restrict_authorization(4, &narrowed), 1);
    assert_eq!(
        *control_signal.borrow(),
        Some(EndReason::CapabilityNarrowed)
    );
    assert_eq!(
        registry.restrict_authorization(4, &narrowed),
        0,
        "asked once"
    );
    let view_only = Narrowed {
        device: "a",
        capability: Capability::ScreenControl,
    };
    assert_eq!(registry.restrict_authorization(5, &view_only), 0);

    // Stop by id, then everything; screen content alone for the indicator.
    let id = handles[0].id();
    assert_eq!(
        registry.stop(&StopSelector::One(id), EndReason::HostStopped),
        1
    );
    assert_eq!(handles[0].stop_reason(), Some(EndReason::HostStopped));
    assert_eq!(registry.end_screen_content(EndReason::HostStopped), 0);
    let content = registry
        .register(session("i", SessionMode::View, true))
        .unwrap();
    assert_eq!(registry.end_screen_content(EndReason::HostStopped), 1);
    assert_eq!(content.stop_reason(), Some(EndReason::HostStopped));
    assert!(registry.stop(&StopSelector::All, EndReason::HostShutdown) >= 3);

    // A reader waits for the next change; the end of a handle is one.
    let (now, _) = registry.sessions();
    let waiter = registry.clone();
    let wait = std::thread::spawn(move || waiter.wait_change(now, Duration::from_secs(10)));
    std::thread::sleep(Duration::from_millis(50));
    drop(content);
    let (changed, views) = wait.join().unwrap();
    assert_ne!(changed, now);
    assert!(views.iter().all(|view| view.device_id != "i"));
    drop(handles);
    drop(first);
    assert!(registry.is_empty());
    assert!(registry.controller().is_none());
}

#[test]
fn screen_task_registry_session_view_json_shape() {
    let registry = Arc::new(ScreenRegistry::default());
    let mut room = session("laptop", SessionMode::Control, true);
    room.principal = principal("laptop", "room");
    let handle = registry.register(room).unwrap();
    handle.update(|view| {
        view.codec = Some("tiles".into());
        view.encoder = Some("tiles".into());
        view.state = SessionState::Streaming;
        view.bitrate_kbps = 1200;
        view.fps = 30;
    });
    let json = serde_json::to_value(&registry.sessions().1[0]).unwrap();
    let keys: Vec<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let mut expected = vec![
        "id",
        "device_id",
        "device_name",
        "relation",
        "room_id",
        "mode",
        "source",
        "codec",
        "encoder",
        "path",
        "started_at",
        "bitrate_kbps",
        "fps",
        "controlling",
        "state",
    ];
    expected.sort_unstable();
    let mut keys = keys;
    keys.sort_unstable();
    assert_eq!(keys, expected);
    assert_eq!(json["mode"], "control");
    assert_eq!(json["state"], "streaming");
    assert_eq!(json["relation"], "room");
    assert_eq!(json["room_id"], "relation-laptop");
}
