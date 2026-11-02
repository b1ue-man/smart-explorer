//! DSK-M1: frames of the shell/surface channel (C5.5) and the S4 viewer
//! target (C4.9).
use std::collections::BTreeMap;

use super::config::{DesktopConfig, SystemItems};
use super::items::{DesktopFile, SystemItem};
use super::layout::{Cell, ItemId, LayoutOp, MonitorLayout};
use super::protocol::{
    check_hello, decode_frames, encode_frame, read_frame, write_frame, ProtocolError, ShellMessage,
    StopReason, SurfaceMessage, SurfaceModel, EXIT_CHANNEL_END, PROTOCOL_VERSION,
};
use super::screen_target::ScreenTarget;
use super::tiles::{project, ShareState, TileInput};
use crate::share::{PeerOpenTarget, ShareProfiles};

const MONITOR: &str = "0123456789abcdef";

fn shell_messages() -> Vec<ShellMessage> {
    let profiles = ShareProfiles::default();
    let (paths, state) = (BTreeMap::new(), ShareState::Live);
    let tiles = project(&TileInput {
        profiles: Ok(&profiles),
        share_state: &state,
        now_secs: 0,
        device_paths: &paths,
        vaults: None,
    });
    let mut layout = MonitorLayout::new(MONITOR, 8).unwrap();
    layout
        .cells
        .insert(ItemId::AllScreens, Cell { col: 0, row: 0 });
    let model = SurfaceModel {
        tiles,
        files: vec![DesktopFile {
            name: "Bericht.md".into(),
            is_dir: false,
            public: false,
            launcher: false,
            hidden: false,
        }],
        system_items: vec![SystemItem::RecycleBin],
        screens: true,
        layout,
    };
    vec![
        ShellMessage::Hello {
            protocol: PROTOCOL_VERSION,
            monitor: MONITOR.into(),
        },
        ShellMessage::Model {
            rev: 7,
            monitor: None,
            model: Box::new(model),
        },
        ShellMessage::Config {
            config: DesktopConfig::initial(SystemItems::initial(None, None)),
        },
        ShellMessage::Stop {
            reason: StopReason::Update,
        },
    ]
}

fn surface_messages() -> Vec<SurfaceMessage> {
    vec![
        SurfaceMessage::Hello {
            protocol: PROTOCOL_VERSION,
            monitor: MONITOR.into(),
            pid: 4242,
            token: None,
        },
        SurfaceMessage::Alive,
        SurfaceMessage::Layout {
            op: LayoutOp::Move {
                item: ItemId::Device {
                    key: "direct~c1".into(),
                },
                monitor: MONITOR.into(),
                col: 2,
                row: 3,
            },
            monitor: None,
        },
        SurfaceMessage::MonitorsChanged,
        SurfaceMessage::WallpaperChanged,
        SurfaceMessage::Connect,
    ]
}

#[test]
fn desktop_task_protocol_frames_roundtrip() {
    // A stream of frames reads back message by message, then ends cleanly.
    let shell = shell_messages();
    let mut stream = Vec::new();
    for message in &shell {
        write_frame(&mut stream, message).unwrap();
    }
    let mut reader = stream.as_slice();
    for message in &shell {
        let read = read_frame::<_, ShellMessage>(&mut reader).unwrap();
        assert_eq!(read.as_ref(), Some(message));
    }
    assert_eq!(read_frame::<_, ShellMessage>(&mut reader), Ok(None));
    let surface = surface_messages();
    let mut stream = Vec::new();
    for message in &surface {
        write_frame(&mut stream, message).unwrap();
    }
    let (decoded, rest) = decode_frames::<SurfaceMessage>(&stream).unwrap();
    assert_eq!(decoded, surface);
    assert_eq!(rest, 0);

    // Frame layout: u32 little-endian length, then UTF-8 JSON.
    let frame = encode_frame(&SurfaceMessage::Alive).unwrap();
    let body = br#"{"type":"alive"}"#;
    assert_eq!(&frame[..4], &(body.len() as u32).to_le_bytes());
    assert_eq!(&frame[4..], body);
    let stop = encode_frame(&ShellMessage::Stop {
        reason: StopReason::Disable,
    })
    .unwrap();
    assert_eq!(&stop[4..], br#"{"type":"stop","reason":"disable"}"#);

    // A truncated frame is an error, in the length and in the body.
    for cut in [1, 3, 5, frame.len() - 1] {
        let mut partial = &frame[..cut];
        assert!(
            matches!(
                read_frame::<_, SurfaceMessage>(&mut partial),
                Err(ProtocolError::Truncated { .. })
            ),
            "cut at {cut}"
        );
    }
    // A length beyond the data allocates nothing up front and is truncated.
    let mut huge = u32::MAX.to_le_bytes().to_vec();
    huge.extend_from_slice(b"{}");
    assert_eq!(
        read_frame::<_, SurfaceMessage>(&mut huge.as_slice()),
        Err(ProtocolError::Truncated {
            expected: u64::from(u32::MAX),
            received: 2
        })
    );
    // A body that is not UTF-8 JSON of a known message is refused.
    let bodies: [&[u8]; 2] = [&[0xff, 0xfe, 0x00], br#"{"type":"unbekannt"}"#];
    for body in bodies {
        let mut bad = (body.len() as u32).to_le_bytes().to_vec();
        bad.extend_from_slice(body);
        assert!(matches!(
            read_frame::<_, SurfaceMessage>(&mut bad.as_slice()),
            Err(ProtocolError::Json(_))
        ));
    }
    // Complete frames are decoded; a partial tail waits for more bytes.
    let mut buffer = encode_frame(&SurfaceMessage::Connect).unwrap();
    buffer.extend_from_slice(&frame[..5]);
    let (messages, rest) = decode_frames::<SurfaceMessage>(&buffer).unwrap();
    assert_eq!(messages, [SurfaceMessage::Connect]);
    assert_eq!(rest, 5);

    // The shell's check of the greeting (GNOME socket with token).
    let hello = |monitor: &str, protocol: u32, token: Option<&str>| SurfaceMessage::Hello {
        protocol,
        monitor: monitor.into(),
        pid: 1,
        token: token.map(str::to_string),
    };
    let token = "00112233445566778899aabbccddeeff";
    assert_eq!(
        check_hello(&hello(MONITOR, 1, None), Some(MONITOR), None),
        Ok(())
    );
    assert_eq!(
        check_hello(&hello("gnome-0", 1, Some(token)), None, Some(token)),
        Ok(())
    );
    for (message, monitor, expected_token) in [
        (hello(MONITOR, 2, None), Some(MONITOR), None),
        (hello("anderer", 1, None), Some(MONITOR), None),
        (hello("gnome-0", 1, None), None, Some(token)),
        (
            hello("gnome-0", 1, Some("ffeeddccbbaa99887766554433221100")),
            None,
            Some(token),
        ),
        (SurfaceMessage::Alive, None, None),
    ] {
        assert!(matches!(
            check_hello(&message, monitor, expected_token),
            Err(ProtocolError::Hello(_))
        ));
    }
    assert_eq!(EXIT_CHANNEL_END, 76);
}

#[test]
fn desktop_task_screen_target_json() {
    let direct = ScreenTarget::new(
        &PeerOpenTarget::Direct {
            contact_id: "c1".into(),
        },
        "device-a",
        false,
    );
    assert_eq!(
        direct.to_arg().unwrap(),
        r#"{"relation":"direct","relation_id":"c1","device_id":"device-a","source":null,"control":false}"#
    );
    let room = ScreenTarget::new(
        &PeerOpenTarget::RoomDevice {
            room_id: "raum-1".into(),
            device_id: "d1".into(),
        },
        "d1",
        true,
    );
    assert_eq!(
        room.to_arg().unwrap(),
        r#"{"relation":"room","relation_id":"raum-1","device_id":"d1","source":null,"control":true}"#
    );
    let parsed: ScreenTarget = serde_json::from_str(&room.to_arg().unwrap()).unwrap();
    assert_eq!(parsed, room);

    // From a tile: Direct peers with a known device, room members always.
    let mut known = contact_profiles();
    let (paths, state) = (BTreeMap::new(), ShareState::Live);
    let tiles = project(&TileInput {
        profiles: Ok(&known),
        share_state: &state,
        now_secs: 0,
        device_paths: &paths,
        vaults: None,
    });
    let target = ScreenTarget::for_tile(&tiles.devices[0], true).unwrap();
    assert_eq!(
        (target.relation.as_str(), target.relation_id.as_str()),
        ("direct", "c1")
    );
    assert_eq!(target.device_id, "device-c1");
    assert!(target.control && target.source.is_none());
    known.direct_contacts[0].remote_device_id = None;
    let unknown = project(&TileInput {
        profiles: Ok(&known),
        share_state: &state,
        now_secs: 0,
        device_paths: &paths,
        vaults: None,
    });
    assert_eq!(ScreenTarget::for_tile(&unknown.devices[0], false), None);
}

fn contact_profiles() -> ShareProfiles {
    let contact = crate::share::DirectContact {
        id: "c1".into(),
        display_name: "Laptop".into(),
        lookup_id: "lookup-c1".into(),
        expected_fingerprint: "fp-c1".into(),
        expected_node_id: "node-c1".into(),
        remote_device_id: Some("device-c1".into()),
        remote_public_key: Some("key-c1".into()),
        auto_connect: true,
        auto_open: false,
        last_seen: None,
        status: crate::share::ShareStatus::Connected,
        last_error: None,
        presence: None,
        access_state: crate::share::DirectAccessState::Accepted,
        request_sent_at: None,
        accepted_at: None,
        accepted_public_key: None,
        lan_candidates: Vec::new(),
        lan_seen_at: None,
        lan_uplink: None,
        relation: Default::default(),
    };
    ShareProfiles {
        direct_contacts: vec![contact],
        ..ShareProfiles::default()
    }
}

/// DSK-M7 (U37-3): a layer process serves every output of the session over
/// one channel, so the model and the layout change carry the output's key.
/// The field stays off the wire when it is `None`, so the frames of a
/// surface of one monitor are the bytes they were, and an old line reads on.
#[test]
fn desktop_task_layer_protocol_model_per_monitor() {
    let model = shell_messages()
        .into_iter()
        .find_map(|message| match message {
            ShellMessage::Model { model, .. } => Some(model),
            _ => None,
        })
        .unwrap();
    let json = |message: &ShellMessage| serde_json::to_value(message).unwrap();

    let plain = ShellMessage::Model {
        rev: 7,
        monitor: None,
        model: model.clone(),
    };
    assert!(json(&plain).get("monitor").is_none());
    let per_monitor = ShellMessage::Model {
        rev: 7,
        monitor: Some("DP-1".into()),
        model: model.clone(),
    };
    assert_eq!(json(&per_monitor)["monitor"], "DP-1");
    assert_eq!(json(&per_monitor)["type"], "model");

    // Both read back from frames; a line of the earlier protocol (without the
    // field) reads as `None`.
    let (decoded, rest) = decode_frames::<ShellMessage>(
        &[
            encode_frame(&plain).unwrap(),
            encode_frame(&per_monitor).unwrap(),
        ]
        .concat(),
    )
    .unwrap();
    assert_eq!(decoded, vec![plain.clone(), per_monitor]);
    assert_eq!(rest, 0);
    let old_line = json(&plain).to_string();
    assert_eq!(
        serde_json::from_str::<ShellMessage>(&old_line).unwrap(),
        plain
    );

    let op = LayoutOp::MoveToMonitor {
        item: ItemId::AllScreens,
        monitor: "HDMI-A-1".into(),
    };
    let from_layer = SurfaceMessage::Layout {
        op: op.clone(),
        monitor: Some("DP-1".into()),
    };
    let from_surface = SurfaceMessage::Layout {
        op: op.clone(),
        monitor: None,
    };
    let value = serde_json::to_value(&from_layer).unwrap();
    assert_eq!(value["monitor"], "DP-1");
    assert!(serde_json::to_value(&from_surface)
        .unwrap()
        .get("monitor")
        .is_none());
    assert_eq!(
        serde_json::from_value::<SurfaceMessage>(value).unwrap(),
        from_layer
    );
    let old_layout = serde_json::to_string(&from_surface).unwrap();
    assert_eq!(
        serde_json::from_str::<SurfaceMessage>(&old_layout).unwrap(),
        from_surface
    );
    // The version stays 1: nothing a surface of one monitor writes changed.
    assert_eq!(PROTOCOL_VERSION, 1);
}
