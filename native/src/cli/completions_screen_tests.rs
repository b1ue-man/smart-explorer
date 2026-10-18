//! Acceptance of the `se screen` completion (SCR-M2): devices by endpoint
//! with relation and state, session ids, and the subcommands and options on
//! the command line.
use clap::CommandFactory;
use clap_complete::CompletionCandidate;

use super::{device_candidates_from, session_candidates_from};
use crate::screen::core::registry::SessionView;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::wire_hello::SessionMode;
use crate::share::{
    DirectContact, ExecGrant, RoomMember, RoomMemberAdmission, RoomMemberFlags, RoomPolicy,
    RoomProfile, ShareProfiles, ShareStatus,
};

const NOW: i64 = 1_760_000_000;

fn values(candidates: &[CompletionCandidate]) -> Vec<String> {
    let value = |c: &CompletionCandidate| c.get_value().to_string_lossy().into_owned();
    candidates.iter().map(value).collect()
}

fn helps(candidates: &[CompletionCandidate]) -> Vec<String> {
    let help = |c: &CompletionCandidate| c.get_help().map(ToString::to_string).unwrap_or_default();
    candidates.iter().map(help).collect()
}

fn contact(id: &str, name: &str) -> DirectContact {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "display_name": name,
        "lookup_id": format!("lookup-{id}"),
        "expected_fingerprint": format!("fp-{id}"),
        "remote_device_id": format!("device-{id}"),
        "auto_connect": false,
        "auto_open": false,
    }))
    .unwrap()
}

fn member(device: &str) -> RoomMember {
    RoomMember {
        device_id: device.into(),
        device_name: device.to_uppercase(),
        fingerprint: format!("fp-{device}"),
        public_key: format!("key-{device}"),
        node_id: format!("key-{device}"),
        relay_url: String::new(),
        candidates: Vec::new(),
        last_seen: None,
        status: ShareStatus::Waiting,
        blocked: false,
        exec: ExecGrant::default(),
        presence: None,
        relation: RoomMemberFlags {
            admission: RoomMemberAdmission::Admitted,
            ..RoomMemberFlags::default()
        },
    }
}

#[test]
fn screen_task_completions_screen() {
    let profiles = ShareProfiles {
        direct_contacts: vec![contact("c1", "Laptop")],
        rooms: vec![RoomProfile {
            id: "room-profile".into(),
            name: "Ops".into(),
            room_id: "room-wire".into(),
            auto_join: true,
            last_seen: None,
            status: ShareStatus::Waiting,
            members: vec![member("tablet")],
            exports: Default::default(),
            policy: RoomPolicy::new_room(),
        }],
        ..ShareProfiles::default()
    };
    let devices = device_candidates_from(&profiles, NOW);
    assert_eq!(
        values(&devices),
        ["share://direct/c1", "share://room/room-wire/tablet"]
    );
    assert_eq!(
        helps(&devices),
        ["Laptop · direct · offline", "Ops / TABLET · room · offline"]
    );

    let session = SessionView {
        id: "3".into(),
        device_id: "device-c1".into(),
        device_name: "Laptop".into(),
        relation: "direct".into(),
        room_id: None,
        mode: SessionMode::View,
        source: "synthetic".into(),
        codec: Some("tiles".into()),
        encoder: Some("tiles".into()),
        path: "direct".into(),
        started_at: NOW,
        bitrate_kbps: 900,
        fps: 30,
        controlling: false,
        state: SessionState::Streaming,
    };
    let ids = session_candidates_from(&[session]);
    assert_eq!(values(&ids), ["3"]);
    assert_eq!(helps(&ids), ["Laptop · view · streaming"]);

    // The command line completes the new command, its subcommands and
    // options.
    let complete = |words: &[&str]| {
        let args = words.iter().map(std::ffi::OsString::from).collect();
        let index = words.len() - 1;
        let candidates =
            clap_complete::engine::complete(&mut crate::cli::Cli::command(), args, index, None)
                .unwrap();
        values(&candidates)
    };
    assert!(complete(&["se", "scr"]).contains(&"screen".to_string()));
    for sub in ["devices", "probe", "bench", "sessions", "stop"] {
        let typed = &sub[..2];
        let found = complete(&["se", "screen", typed]);
        assert!(found.contains(&sub.to_string()), "{sub}: {found:?}");
    }
    for (command, typed, option) in [
        ("bench", "--lo", "--local"),
        ("bench", "--co", "--codec"),
        ("bench", "--si", "--size"),
        ("bench", "--se", "--seconds"),
        ("devices", "--pr", "--probe"),
        ("devices", "--ro", "--room"),
        ("stop", "--al", "--all"),
        ("sessions", "--js", "--json"),
    ] {
        let found = complete(&["se", "screen", command, typed]);
        assert!(found.contains(&option.to_string()), "{option}: {found:?}");
    }
    let codecs = complete(&["se", "screen", "bench", "--local", "--codec", ""]);
    for codec in ["auto", "h264", "tiles"] {
        assert!(codecs.contains(&codec.to_string()), "{codec}: {codecs:?}");
    }
}
