//! Tests of the Android viewer (SCR-M9, U28) without a device: the shapes of
//! the bridge methods and events, the session's input rules and the keys the
//! app looks its texts up by. The queue behind `screenPull` is tested in
//! `screen/os/android/viewer_sink_tests.rs`; the decoder and the touch mapping
//! run in `ScreenTaskTest.kt` and the JVM tests under `ui/screen/`.
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use super::json_shapes::{accepted_fields, devices_json, event_value, stats_fields};
use super::run::{open_for_tests, SendFrame};
use super::*;
use crate::screen::core::session_state::{EndReason, SessionState};
use crate::screen::core::wire_control::ControlBlock;
use crate::screen::core::wire_hello::ScreenErrorCode;
use crate::screen::os::viewer_sink::PULL_ENDED;
use crate::share::access_grant::CapabilityDecision;
use crate::share::{DirectAccessState, DirectContact, ShareStatus};

const NOW: i64 = 1_800_000_000;

fn contact() -> DirectContact {
    DirectContact {
        id: "contact-a".into(),
        display_name: "Laptop".into(),
        lookup_id: "lookup-a".into(),
        expected_fingerprint: "fp-a".into(),
        expected_node_id: "node-a".into(),
        remote_device_id: Some("device-a".into()),
        remote_public_key: Some("key-a".into()),
        auto_connect: true,
        auto_open: false,
        last_seen: Some(NOW - 60),
        status: ShareStatus::Connected,
        last_error: None,
        presence: None,
        access_state: DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: None,
        accepted_public_key: None,
        lan_candidates: Vec::new(),
        lan_seen_at: None,
        lan_uplink: None,
        relation: Default::default(),
    }
}

#[allow(clippy::field_reassign_with_default)]
fn profiles() -> ShareProfiles {
    let member = |id: &str, name: &str, status: &str, blocked: bool| {
        json!({
            "device_id": id, "device_name": name, "fingerprint": "fp", "public_key": "pk",
            "candidates": [], "last_seen": null, "status": status, "blocked": blocked,
        })
    };
    let room = json!({
        "id": "room-profile", "name": "Büro", "room_id": "room-1", "auto_join": true,
        "last_seen": null,
        "members": [
            member("device-b", "Tablet", "Connected", false),
            member("device-c", "Gesperrt", "Connected", true),
            member("device-me", "Dieses Handy", "Connected", false),
        ],
    });
    let mut profiles = ShareProfiles::default();
    profiles.direct_contacts = vec![contact()];
    profiles.rooms = vec![serde_json::from_value(room).unwrap()];
    profiles
}

#[test]
fn screen_task_android_view_bridge_shapes() {
    // The viewer answers `screen.view*`, `devices`, `probe` and `windows`
    // only; the host methods stay with `screen_host`. (`screen.devices` reads the
    // stored profiles of the machine, so only its JSON is tested.)
    for method in ["screen.hostPrereqs", "screen.hostStop", "screen.unknown"] {
        assert!(handle(method, &json!({})).is_none(), "{method}");
    }
    for method in [
        "screen.probe",
        "screen.windows",
        "screen.viewOpen",
        "screen.viewSetMode",
        "screen.viewInput",
        "screen.viewControl",
        "screen.viewListWindows",
        "screen.viewKeyframe",
        "screen.viewClose",
    ] {
        assert!(
            handle(method, &json!({ "session": 0 })).is_some(),
            "{method}"
        );
    }
    // Devices: Direct devices with a device ID, then the admitted room
    // members except this phone; the probe results fill the unknowns.
    let mut probes = BTreeMap::new();
    probes.insert(
        "direct/contact-a/device-a".to_string(),
        ProbeFacts {
            supported: true,
            view: CapabilityDecision::Ask,
            control: CapabilityDecision::Off,
        },
    );
    let all = devices_json(&profiles(), None, "device-me", NOW, &probes);
    let list = all["devices"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(
        list[0],
        json!({
            "relation": "direct", "relationId": "contact-a", "deviceId": "device-a",
            "name": "Laptop", "roomId": null, "roomName": null, "online": true,
            "supported": true, "view": "ask", "control": "off",
        })
    );
    assert_eq!(list[1]["relation"], "room");
    assert_eq!(list[1]["relationId"], "room-profile");
    assert_eq!(list[1]["deviceId"], "device-b");
    assert_eq!(list[1]["roomId"], "room-profile");
    assert_eq!(list[1]["roomName"], "Büro");
    assert!(list[1]["supported"].is_null() && list[1]["view"].is_null());
    let room = devices_json(&profiles(), Some("room-1"), "device-me", NOW, &probes);
    assert_eq!(room["devices"].as_array().unwrap().len(), 1);
    let none = devices_json(&profiles(), Some("missing"), "device-me", NOW, &probes);
    assert!(none["devices"].as_array().unwrap().is_empty());
    // Events: the envelope and the fields of `accepted` and `stats`.
    let event = event_value(4, "state", json!({ "state": "streaming", "reason": null }));
    assert_eq!(
        event,
        json!({ "type": "screenView", "session": 4, "kind": "state",
                "state": "streaming", "reason": null })
    );
    let hello: crate::screen::core::wire_hello::ScreenHelloOk = serde_json::from_value(json!({
        "authorization": { "view": "allow", "control": "ask" }, "platform": "android",
        "encoders": [{ "codec": "h264", "profile": "high", "hardware": true, "max_width": 1920 }],
        "input": { "touch": true, "toolbar_keys": [101], "actions": ["quick_settings"] },
        "monitors": [{ "id": "0", "name": "", "x": 0, "y": 0, "width": 1080, "height": 2400 }],
        "limits": ["still_max_3_per_second"], "approval": "once",
    }))
    .unwrap();
    let accepted = accepted_fields(&hello, "relay");
    assert_eq!(accepted["path"], "relay");
    assert_eq!(accepted["platform"], "android");
    assert_eq!(
        accepted["authorization"],
        json!({ "view": "allow", "control": "ask" })
    );
    assert_eq!(accepted["encoders"][0]["maxWidth"], 1920);
    assert_eq!(accepted["input"]["toolbarKeys"], json!([101]));
    assert_eq!(accepted["input"]["actions"], json!(["quick_settings"]));
    assert!(accepted["monitors"].is_array() && accepted["limits"].is_array());
    let stats = stats_fields(&crate::screen::core::wire_control::HostStats {
        target_kbps: 6000,
        sent_kbps: 5000,
        fps: 30,
        path: crate::screen::core::rate_control::PathKind::Relay,
        keyframes: 2,
        dropped_before_encode: 1,
        stream_resets: 0,
        unacked_bytes: 10,
        frame_timing: None,
        chain: None,
        stations: None,
    });
    assert_eq!(stats["targetKbps"], 6000);
    assert_eq!(stats["streamResets"], 0);
    assert_eq!(stats["path"], "relay");
    assert!(stats["rttMs"].is_null());
    // Input: a key may be named; more than 256 events are refused.
    let named = input_event(&json!({ "type": "key", "key": "a", "down": true })).unwrap();
    assert_eq!(
        named,
        InputEvent::Key {
            hid_usage: 4,
            down: true
        }
    );
    let touch = input_event(&json!({ "type": "touch", "id": 1, "phase": "move", "x": 5, "y": 6 }));
    assert!(touch.is_ok());
    assert!(input_event(&json!({ "type": "key", "key": "nonsense", "down": true })).is_err());
    assert!(input_event(&json!({ "type": "bogus" })).is_err());
    let events =
        |n: usize| json!({ "events": vec![json!({ "type": "pointer_abs", "x": 1, "y": 2 }); n] });
    assert_eq!(parse_events(&events(256)).unwrap().len(), 256);
    assert_eq!(parse_events(&events(257)).unwrap_err().kind, "invalid");
    // Errors keep the contract: an unknown session is `not_found`, `viewClose` is idempotent.
    let err = handle(
        "screen.viewSetMode",
        &json!({ "session": 4_000_000, "mode": "full" }),
    );
    assert_eq!(err.unwrap().unwrap_err().kind, "not_found");
    let closed = handle("screen.viewClose", &json!({ "session": 4_000_000 }));
    assert_eq!(closed.unwrap().unwrap(), json!({}));
    assert_eq!(
        handle("screen.viewClose", &json!({}))
            .unwrap()
            .unwrap_err()
            .kind,
        "invalid"
    );
}

#[test]
fn screen_task_android_view_input_needs_control() {
    let sent: Arc<Mutex<Vec<ViewerFrame>>> = Arc::default();
    let record_to = sent.clone();
    let send: SendFrame = Arc::new(move |frame: &ViewerFrame| {
        record_to.lock().unwrap().push(frame.clone());
        Ok(())
    });
    let session = open_for_tests(SessionMode::Control, Some(send));
    let id = session.id;
    let key = json!({ "type": "key", "key": "a", "down": true });
    let call = |method: &str, args: Value| handle(method, &args).unwrap();
    let input = json!({ "session": id, "events": [key] });
    // The host has not allowed control: nothing is sent.
    assert_eq!(
        call("screen.viewInput", input.clone()).unwrap_err().kind,
        "permission"
    );
    let take = json!({ "session": id, "action": "take" });
    assert_eq!(
        call("screen.viewControl", take.clone()).unwrap_err().kind,
        "permission"
    );
    assert!(sent.lock().unwrap().is_empty());
    // Allowed: the events go out in one frame and are tracked as pressed.
    session.control_allowed.store(true, Ordering::SeqCst);
    assert_eq!(
        call("screen.viewInput", input).unwrap(),
        json!({ "sent": 1 })
    );
    call("screen.viewControl", take).unwrap();
    assert_eq!(sent.lock().unwrap().len(), 2);
    // Closing releases the key and ends the queue.
    assert_eq!(
        call("screen.viewClose", json!({ "session": id })).unwrap(),
        json!({})
    );
    let frames = sent.lock().unwrap().clone();
    assert_eq!(
        frames.last(),
        Some(&ViewerFrame::Input(vec![InputEvent::Key {
            hid_usage: 4,
            down: false
        }]))
    );
    let mut dst = [0u8; 32];
    assert_eq!(
        screen_pull(id, &mut dst, Duration::from_millis(5)),
        PULL_ENDED
    );
    assert_eq!(
        call("screen.viewClose", json!({ "session": id })).unwrap(),
        json!({})
    );
}

fn all_keys() -> String {
    use SessionState::*;
    let states = [
        Connecting,
        WaitingForApproval,
        WaitingForPortal,
        Starting,
        Streaming,
        SecureDesktop,
        WindowMinimized,
        ViewerHidden,
        HostLocked,
        SessionInactive,
        NoDisplay,
        Ended,
        Unknown,
    ];
    // A new state breaks this match and with it the test (add its text).
    states.iter().for_each(|state| match state {
        Connecting | WaitingForApproval | WaitingForPortal | Starting | Streaming
        | SecureDesktop | WindowMinimized | ViewerHidden | HostLocked | SessionInactive
        | NoDisplay | Ended | Unknown => {}
    });
    let reasons = [
        EndReason::ViewerClosed,
        EndReason::HostStopped,
        EndReason::CapabilityNarrowed,
        EndReason::GrantExpired,
        EndReason::HostShutdown,
        EndReason::UpdateStop,
        EndReason::ConnectionLost,
        EndReason::Failed,
        EndReason::SessionChanged,
    ];
    reasons.iter().for_each(|reason| match reason {
        EndReason::ViewerClosed
        | EndReason::HostStopped
        | EndReason::CapabilityNarrowed
        | EndReason::GrantExpired
        | EndReason::HostShutdown
        | EndReason::UpdateStop
        | EndReason::ConnectionLost
        | EndReason::Failed
        | EndReason::SessionChanged => {}
    });
    let blocks = [
        ControlBlock::ElevatedWindow,
        ControlBlock::SecureDesktop,
        ControlBlock::ControlUnavailable,
        ControlBlock::UnsupportedOnHost,
        ControlBlock::Other,
    ];
    blocks.iter().for_each(|block| match block {
        ControlBlock::ElevatedWindow
        | ControlBlock::SecureDesktop
        | ControlBlock::ControlUnavailable
        | ControlBlock::UnsupportedOnHost
        | ControlBlock::Other => {}
    });
    let mut out = String::new();
    for state in states {
        out.push_str(&format!("state:{}\n", state.key()));
    }
    for code in ScreenErrorCode::ALL {
        out.push_str(&format!("error:{}\n", code.key()));
    }
    for reason in reasons {
        out.push_str(&format!("end:{}\n", reason.key()));
    }
    // Keys the relay adds to the reasons of `EndReason`.
    out.push_str("end:ended\nend:connection_failed\n");
    for block in blocks {
        let key = serde_json::to_value(block).unwrap();
        out.push_str(&format!("block:{}\n", key.as_str().unwrap()));
    }
    out
}

/// The list the Kotlin test `screen_task_android_texts_match_desktop` reads.
#[test]
fn screen_task_android_screen_keys_file_current() {
    let file =
        include_str!("../../../../../../../../android/app/src/test/resources/screen_keys.txt");
    let expected = all_keys();
    assert_eq!(
        file, expected,
        "android/app/src/test/resources/screen_keys.txt must hold:\n{expected}"
    );
}

/// The key names of `ScreenInputMapper.keyName` (Kotlin) are names the core knows.
#[test]
fn screen_task_android_key_names_known() {
    let mut names: Vec<String> = ('a'..='z').map(String::from).collect();
    names.extend(('0'..='9').map(String::from));
    names.extend((1..=12).map(|number| format!("f{number}")));
    names.extend(
        "up down left right enter kp_enter backspace delete tab escape space home end page_up \
         page_down insert minus equal bracket_left bracket_right backslash semicolon quote grave \
         comma period slash ctrl_left ctrl_right shift_left shift_right alt_left alt_right \
         super_left super_right"
            .split_whitespace()
            .map(String::from),
    );
    for name in names {
        assert!(key_usage(&name).is_some(), "the core knows no key {name}");
        let event = json!({ "type": "key", "key": name, "down": true });
        assert!(
            matches!(input_event(&event), Ok(InputEvent::Key { .. })),
            "{name}"
        );
    }
}

/// „Codec: Scharf (GPU)“ of the app reaches the core as the setting `sharp`
/// with its `SET_MODE` bit, „Dekodierer“ as chosen (user rule of
/// 2026-11-18: every choice is used exactly as made); an unknown codec is an
/// error, not the automatic one.
#[test]
fn wahlgilt_codec_sharp_reaches_the_core() {
    use crate::screen::core::negotiate::{CodecSetting, DecoderPreference};
    let (codec, decoder) =
        super::codecs::codec_args(&json!({ "codec": "sharp", "decoder": "hardware" })).unwrap();
    assert_eq!(codec, Some(CodecSetting::Sharp));
    assert_eq!(decoder, Some(DecoderPreference::Hardware));
    assert!(CodecSetting::Sharp.sharpness() && !CodecSetting::Sharp.lossless());
    let unknown = super::codecs::codec_args(&json!({ "codec": "gpu" }));
    assert_eq!(unknown.unwrap_err().kind, "invalid");
}

/// „Höchste Datenrate“ below the host's floor (0,15 Mbit/s) is refused with
/// its reason instead of being raised by the host without a word; 0 stays
/// „automatisch“ in `viewSetMode`.
#[test]
fn wahlgilt_max_rate_below_the_floor_is_refused() {
    use super::codecs::max_rate_arg;
    let floor = crate::screen::core::rate_control::MIN_BITRATE_BPS;
    assert_eq!(max_rate_arg(&json!({}), false).unwrap(), None);
    let at_floor = json!({ "maxRate": floor });
    assert_eq!(max_rate_arg(&at_floor, false).unwrap(), Some(Some(floor)));
    assert_eq!(max_rate_arg(&at_floor, true).unwrap(), Some(Some(floor)));
    let below = json!({ "maxRate": floor - 1 });
    assert_eq!(max_rate_arg(&below, false).unwrap_err().kind, "invalid");
    assert_eq!(max_rate_arg(&below, true).unwrap_err().kind, "invalid");
    let zero = json!({ "maxRate": 0 });
    assert_eq!(max_rate_arg(&zero, false).unwrap_err().kind, "invalid");
    assert_eq!(max_rate_arg(&zero, true).unwrap(), Some(None));
    let negative = json!({ "maxRate": -1 });
    assert_eq!(max_rate_arg(&negative, true).unwrap_err().kind, "invalid");
}

/// The app's „Monitor“/„Auflösung“ line reads what the host did (`accepted`,
/// with `requested`, `adjusted`, `source`) and does now (`stats`).
#[test]
fn wahlgilt_accepted_and_stats_carry_the_virtual_display_report() {
    let hello: crate::screen::core::wire_hello::ScreenHelloOk = serde_json::from_value(json!({
        "authorization": { "view": "allow", "control": "off" }, "platform": "windows",
        "encoders": [], "monitors": [], "limits": [], "approval": "once",
        "virtual_display": {
            "state": "unsupported", "width": 0, "height": 0, "driver": "windows",
            "reason": "driver_missing", "requested": { "width": 2560, "height": 1440 },
            "adjusted": "driver_limit", "source": "monitor:2", "source_chosen": "monitor:1",
        },
    }))
    .unwrap();
    let accepted = accepted_fields(&hello, "direct");
    assert_eq!(accepted["virtualDisplay"]["state"], "unsupported");
    assert_eq!(accepted["virtualDisplay"]["reason"], "driver_missing");
    assert_eq!(accepted["virtualDisplay"]["driver"], "windows");
    assert_eq!(accepted["virtualDisplay"]["requested"]["width"], 2560);
    assert_eq!(accepted["virtualDisplay"]["requested"]["height"], 1440);
    assert_eq!(accepted["virtualDisplay"]["adjusted"], "driver_limit");
    assert_eq!(accepted["virtualDisplay"]["source"], "monitor:2");
    assert_eq!(accepted["virtualDisplay"]["sourceChosen"], "monitor:1");
    let stats = stats_fields(&crate::screen::core::wire_control::HostStats {
        target_kbps: 6000,
        sent_kbps: 5000,
        fps: 30,
        path: crate::screen::core::rate_control::PathKind::Relay,
        keyframes: 0,
        dropped_before_encode: 0,
        stream_resets: 0,
        unacked_bytes: 0,
        frame_timing: None,
        chain: Some(crate::screen::core::frame_timing::ChainReport {
            virtual_display: Some(
                "active:1920x1080:windows;requested=2560x1440;adjusted=driver_limit;source=monitor:2"
                    .into(),
            ),
            ..Default::default()
        }),
        stations: None,
    });
    assert_eq!(
        stats["chain"]["virtualDisplay"],
        "active:1920x1080:windows;requested=2560x1440;adjusted=driver_limit;source=monitor:2"
    );
}

/// The app's decoder list goes to the host through the core's `chosen_decoders`: under a
/// named codec it holds that codec alone, no tiles (the host reads the choice from it and
/// never replaces it); under „Automatisch“ the tiles stay the last fallback.
#[test]
fn wahlgilt_named_codec_offers_no_tiles_to_the_host() {
    use crate::screen::core::negotiate::CodecKind;
    let request = |codec: &str| {
        let args = json!({
            "codec": codec,
            "decoders": [
                { "codec": "h264", "profile": "high", "hardware": true },
                { "codec": "hevc", "profile": "main", "hardware": true },
                { "codec": "tiles" },
            ],
        });
        let (codec, decoder) = super::codecs::codec_args(&args).unwrap();
        crate::daemon::StreamRequest {
            expected_node: None,
            mode: SessionMode::View,
            source: None,
            decoders: super::codecs::decoders_arg(&args, SessionMode::View).unwrap(),
            display_size: None,
            frame_capabilities: None,
            max_fps: 30,
            max_rate_bps: None,
            codec,
            decoder,
            gop_continue: false,
            gop_continue_decoders: Vec::new(),
            virtual_display: None,
            delivery: None,
        }
        .chosen_decoders()
    };
    let kinds = |codec: &str| -> Vec<CodecKind> {
        request(codec).iter().map(|offer| offer.codec).collect()
    };
    assert_eq!(kinds("hevc"), vec![CodecKind::Hevc]);
    assert_eq!(kinds("h264"), vec![CodecKind::H264]);
    assert!(kinds("auto").contains(&CodecKind::Tiles));
    assert!(!kinds("sharp").contains(&CodecKind::Av1));
    assert_eq!(kinds("lossless"), vec![CodecKind::Tiles]);
}
