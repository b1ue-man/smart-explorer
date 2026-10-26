use super::*;
use crate::screen::core::input_model::{
    decode_batch, encode_batch, InputCapabilities, InputEvent, PointerButton, SystemAction,
    TouchPhase, KEY_PLAY_PAUSE, MAX_INPUT_EVENTS, MAX_TEXT_BYTES, TOOLBAR_KEYS,
};
use crate::screen::core::negotiate::{CodecKind, CodecOffer, H264Profile};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::rate_control::PathKind;
use crate::screen::core::session_state::SessionState;
use crate::screen::core::tiles_codec::{TileDecoder, MAX_TILES_RECORD_BYTES};
use crate::screen::core::window_list::{
    AppEntry, ListSource, MonitorInfo, SourceId, WindowEntry, WindowList,
};
use crate::screen::core::wire_binary::{
    read_frame_record, write_frame_record, FrameRecord, GopHeader, StreamKind, FLAG_KEYFRAME,
    MAX_RECORD_BYTES, RECORD_HEADER_BYTES,
};
use crate::screen::core::wire_control::*;
use crate::screen::core::wire_hello::*;
use crate::share::access_grant::{Capability, CapabilityDecision};

fn run<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn server_hello() -> ScreenServerHello {
    ScreenServerHello::new(
        "c".repeat(32),
        "host-device".into(),
        "host-key".into(),
        "host-fingerprint".into(),
        "host-node".into(),
    )
}

fn client_hello() -> ScreenClientHello {
    ScreenClientHello {
        family_version: None,
        icon_version: None,
        metadata_only: false,
        frame_capabilities: None,
        protocol_version: SCREEN_PROTOCOL_VERSION,
        relation_kind: "direct".into(),
        relation_id: "lookup".into(),
        device_id: "viewer".into(),
        device_name: "Laptop".into(),
        public_key: "viewer-key".into(),
        fingerprint: "viewer-fingerprint".into(),
        node_id: "viewer-node".into(),
        client_nonce: "n".repeat(32),
        proof: "proof".into(),
        mode: SessionMode::Control,
        source: Some(SourceId::Window("0x2a".into())),
        decoders: vec![
            CodecOffer::h264(H264Profile::High, true),
            CodecOffer::tiles(),
        ],
        display_size: Some(FrameSize::new(2560, 1440)),
        max_fps: 60,
    }
}

fn replies() -> Vec<HelloReply> {
    let accepted = ScreenHelloOk {
        family_version: None,
        icon_version: None,
        frame_contract: None,
        authorization: ScreenAuthorization {
            view: CapabilityDecision::Allow,
            control: CapabilityDecision::Ask,
        },
        platform: HostPlatform::Linux,
        encoders: vec![
            CodecOffer::h264(H264Profile::ConstrainedBaseline, false).with_max(FrameSize::MAX)
        ],
        input: InputCapabilities {
            pointer: true,
            keyboard: true,
            text: true,
            touch: false,
            actions: vec![SystemAction::Home],
            toolbar_keys: TOOLBAR_KEYS.to_vec(),
        },
        monitors: vec![MonitorInfo {
            id: "0".into(),
            name: "DP-1".into(),
            x: -1920,
            y: 0,
            width: 2560,
            height: 1440,
            primary: true,
        }],
        limits: vec![LIMIT_NO_KEY_EVENTS.into()],
        approval: ApprovalKind::Once,
    };
    vec![
        HelloReply::Pending(ScreenApprovalPending {
            capability: Capability::ScreenControl,
        }),
        HelloReply::Accepted(accepted),
        HelloReply::Refused(ScreenHelloError::new(ScreenErrorCode::RightNotGranted)),
    ]
}

fn viewer_frames() -> Vec<ViewerFrame> {
    vec![
        ViewerFrame::SetMode(SetMode {
            mode: StreamMode::Full,
            source: Some(SourceId::Monitor("1".into())),
            size: Some(FrameSize::new(1280, 720)),
            fps: Some(30),
            sharpness: true,
            lossless: false,
        }),
        ViewerFrame::Input(vec![
            InputEvent::PointerAbs { x: 0, y: 65_535 },
            InputEvent::Button {
                button: PointerButton::Forward,
                down: true,
            },
            InputEvent::Wheel { dx: -120, dy: 240 },
            InputEvent::Key {
                hid_usage: 0x04,
                down: true,
            },
            InputEvent::Text {
                text: "Grüße ✓".into(),
            },
            InputEvent::Touch {
                id: 2,
                phase: TouchPhase::Move,
                x: 1,
                y: 2,
            },
            InputEvent::Action {
                action: SystemAction::QuickSettings,
            },
        ]),
        ViewerFrame::FrameAck(FrameAck {
            gop_id: 3,
            frame_id: 99,
            received_us: 123_456,
        }),
        ViewerFrame::KeyframeRequest(KeyframeRequest {
            reason: KeyframeReason::DecodeError,
            gop_id: Some(3),
        }),
        ViewerFrame::Ping(Ping {
            sequence: 5,
            sent_us: 777,
        }),
        ViewerFrame::ListWindows,
        ViewerFrame::Snapshot(SnapshotRequest {
            source: Some(SourceId::Synthetic),
            max_size: Some(FrameSize::new(640, 360)),
        }),
        ViewerFrame::ControlRequest(ControlRequest {
            action: ControlAction::Take,
        }),
        ViewerFrame::PreviewAck(FrameAck {
            gop_id: 4,
            frame_id: 100,
            received_us: 654_321,
        }),
    ]
}

fn host_frames() -> Vec<HostFrame> {
    vec![
        HostFrame::State(StateReport {
            state: SessionState::Streaming,
            reason: None,
            stream: Some(StreamInfo {
                codec: CodecKind::H264,
                profile: Some(H264Profile::High),
                size: FrameSize::new(1920, 1080),
                encoder: "openh264".into(),
                hardware: false,
            }),
        }),
        HostFrame::Pong(Pong {
            sequence: 5,
            ping_sent_us: 777,
            received_us: 1_000_000,
            sent_us: 1_000_050,
        }),
        HostFrame::Windows(WindowList {
            windows: vec![WindowEntry {
                id: "0x2a".into(),
                title: "Editor".into(),
                app: "gedit".into(),
                pid: Some(42),
                monitor: Some("0".into()),
                minimized: false,
                focused: true,
            }],
            apps: vec![app_entry()],
            list_source: ListSource::Ewmh,
            limits: Vec::new(),
        }),
        HostFrame::Stats(HostStats {
            target_kbps: 6000,
            sent_kbps: 5800,
            fps: 30,
            path: PathKind::Relay,
            keyframes: 2,
            dropped_before_encode: 1,
            stream_resets: 0,
            unacked_bytes: 4096,
        }),
        HostFrame::ControlState(ControlState {
            holder: Some(ControlHolder {
                device_id: "viewer".into(),
                device_name: "Laptop".into(),
            }),
            blocked: Some(ControlBlock::ElevatedWindow),
        }),
        HostFrame::Error(ScreenWireError::new(
            ScreenErrorCode::HostShutdown,
            Some("daemon stopped".into()),
        )),
    ]
}

fn app_entry() -> AppEntry {
    AppEntry {
        app_id: "org.gnome.gedit".into(),
        name: "Text Editor".into(),
        pid: Some(42),
    }
}

fn video_header() -> GopHeader {
    GopHeader {
        family_layout_revision: None,
        frame_contract: None,
        gop_id: 0x0102_0304_0506_0708,
        kind: StreamKind::Video,
        codec: CodecKind::H264,
        profile: Some(H264Profile::High),
        size: FrameSize::new(1920, 1080),
    }
}

fn tiles_header() -> GopHeader {
    GopHeader {
        family_layout_revision: None,
        frame_contract: None,
        gop_id: 9,
        kind: StreamKind::Preview,
        codec: CodecKind::Tiles,
        profile: None,
        size: FrameSize::new(320, 180),
    }
}

fn record(frame_id: u64, flags: u8, payload: Vec<u8>) -> FrameRecord {
    FrameRecord {
        frame_id,
        capture_ts_us: 1_000,
        flags,
        payload,
    }
}

#[test]
fn screen_task_wire_roundtrip_all_tags() {
    run(async {
        // Handshake: tags 1–5.
        let mut buffer = Vec::new();
        send_server_hello(&mut buffer, &server_hello())
            .await
            .unwrap();
        send_client_hello(&mut buffer, &client_hello())
            .await
            .unwrap();
        for reply in replies() {
            send_hello_reply(&mut buffer, &reply).await.unwrap();
        }
        let mut reader = buffer.as_slice();
        assert_eq!(reader[0], SERVER_HELLO);
        assert_eq!(
            recv_server_hello(&mut reader).await.unwrap(),
            server_hello()
        );
        assert_eq!(reader[0], CLIENT_HELLO);
        assert_eq!(
            recv_client_hello(&mut reader).await.unwrap(),
            client_hello()
        );
        for (reply, tag) in replies()
            .into_iter()
            .zip([APPROVAL_PENDING, HELLO_OK, HELLO_ERROR])
        {
            assert_eq!(reader[0], tag);
            assert_eq!(recv_hello_reply(&mut reader).await.unwrap(), reply);
        }
        assert!(reader.is_empty());

        // Viewer to host: tags 16–24; host to viewer: 32–34 and 36–38.
        let mut buffer = Vec::new();
        for frame in viewer_frames() {
            send_viewer_frame(&mut buffer, &frame).await.unwrap();
        }
        let mut reader = buffer.as_slice();
        for (frame, tag) in viewer_frames().into_iter().zip(16u8..=24) {
            assert_eq!(reader[0], tag);
            assert_eq!(recv_viewer_frame(&mut reader).await.unwrap(), frame);
        }
        let mut buffer = Vec::new();
        for frame in host_frames() {
            send_host_frame(&mut buffer, &frame).await.unwrap();
        }
        let mut reader = buffer.as_slice();
        for (frame, tag) in host_frames().into_iter().zip([32u8, 33, 34, 36, 37, 38]) {
            assert_eq!(reader[0], tag);
            assert_eq!(recv_host_frame(&mut reader).await.unwrap(), frame);
        }
        assert!(reader.is_empty());

        // Each direction refuses the other's tags and the free tag 35.
        assert!(decode_viewer_frame(STATE, b"{}").is_err());
        assert!(decode_host_frame(SET_MODE, br#"{"mode":"full"}"#).is_err());
        assert!(decode_host_frame(35, b"{}").is_err());
        assert!(decode_viewer_frame(HELLO_OK, b"{}").is_err());

        // Values of a newer host decode instead of ending the session.
        let state = decode_host_frame(STATE, br#"{"state":"future_state"}"#).unwrap();
        assert!(matches!(state, HostFrame::State(report) if report.state == SessionState::Unknown));
        let control = decode_host_frame(CONTROL_STATE, br#"{"blocked":"future"}"#).unwrap();
        assert!(matches!(
            control,
            HostFrame::ControlState(ControlState {
                blocked: Some(ControlBlock::Other),
                ..
            })
        ));

        // Big-endian layouts, byte for byte, on little-endian targets too.
        let mut frame = Vec::new();
        let ack = FrameAck {
            gop_id: 0x0102_0304_0506_0708,
            frame_id: 0x1112_1314_1516_1718,
            received_us: 0x2122_2324_2526_2728,
        };
        send_viewer_frame(&mut frame, &ViewerFrame::FrameAck(ack))
            .await
            .unwrap();
        let mut expected = vec![FRAME_ACK, 0, 0, 0, 24];
        for start in [0x01u8, 0x11, 0x21] {
            expected.extend((0..8).map(|offset| start + offset));
        }
        assert_eq!(frame, expected);
        let batch = encode_batch(&[
            InputEvent::PointerAbs {
                x: 0x1234,
                y: 0xABCD,
            },
            InputEvent::Key {
                hid_usage: KEY_PLAY_PAUSE,
                down: true,
            },
        ])
        .unwrap();
        assert_eq!(batch, [0, 2, 1, 0x12, 0x34, 0xAB, 0xCD, 4, 0x80, 0xCD, 1]);
        let head = video_header().encode().unwrap();
        assert_eq!(
            head,
            [b'S', b'C', b'R', b'V', 1, 1, 2, 3, 4, 5, 6, 7, 8, 1, 1, 2, 0x07, 0x80, 0x04, 0x38]
        );
    });
}

#[test]
fn screen_task_frame_limits_reject_oversize() {
    run(async {
        // A head announcing more than the reader accepts fails as invalid data,
        // not as a short read: the payload is neither read nor allocated.
        let mut head = vec![SET_MODE];
        head.extend_from_slice(&u32::MAX.to_be_bytes());
        let error = recv_viewer_frame(&mut head.as_slice()).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let mut head = vec![WINDOWS];
        head.extend_from_slice(&(MAX_WINDOWS_BYTES as u32 + 1).to_be_bytes());
        let error = recv_host_frame(&mut head.as_slice()).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        // 1 MiB is for `WINDOWS` only; any other host frame stays within 64 KiB.
        let mut stats = vec![STATS];
        stats.extend_from_slice(&(MAX_CONTROL_BYTES as u32 + 1).to_be_bytes());
        stats.resize(5 + MAX_CONTROL_BYTES + 1, b' ');
        let error = recv_host_frame(&mut stats.as_slice()).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        // Writers refuse before they write a byte.
        let mut sink = Vec::new();
        assert!(
            write_frame(&mut sink, STATS, &vec![b' '; MAX_CONTROL_BYTES + 1])
                .await
                .is_err()
        );
        let crowded = vec![InputEvent::PointerAbs { x: 1, y: 1 }; MAX_INPUT_EVENTS + 1];
        assert!(send_viewer_frame(&mut sink, &ViewerFrame::Input(crowded))
            .await
            .is_err());
        let long_text = InputEvent::Text {
            text: "a".repeat(MAX_TEXT_BYTES + 1),
        };
        assert!(encode_batch(&[long_text]).is_err());
        // R15c: no entry count; more than the former 4096 entries go out
        // whole while the frame holds them.
        let crowded_list = WindowList {
            windows: Vec::new(),
            apps: vec![app_entry(); 4097],
            list_source: ListSource::SystemdUnits,
            limits: Vec::new(),
        };
        let mut crowded_frame = Vec::new();
        let list = HostFrame::Windows(crowded_list.clone());
        send_host_frame(&mut crowded_frame, &list).await.unwrap();
        assert!(crowded_frame.len() <= 5 + MAX_WINDOWS_BYTES);
        assert_eq!(crowded_list.bounded().apps.len(), 4097);
        // JSON escaping (6 bytes per control character) cannot push a
        // bounded list over 1 MiB: it keeps fewer entries instead.
        let escaped = WindowEntry {
            id: "w".into(),
            title: "\u{1}".repeat(300),
            app: "a".repeat(100),
            pid: Some(u32::MAX),
            monitor: Some("m".repeat(100)),
            minimized: true,
            focused: true,
        };
        let escaped = WindowList {
            windows: vec![escaped; 4096],
            apps: vec![app_entry(); 8],
            list_source: ListSource::KdePlasmaWindowManagement,
            limits: vec!["\u{1}".repeat(100); 20],
        }
        .bounded();
        assert!(!escaped.windows.is_empty() && escaped.windows.len() < 4096);
        let mut frame = Vec::new();
        send_host_frame(&mut frame, &HostFrame::Windows(escaped))
            .await
            .unwrap();
        assert!(frame.len() <= 5 + MAX_WINDOWS_BYTES);
        let mut hello = client_hello();
        hello.decoders = vec![CodecOffer::tiles(); 33];
        assert!(send_client_hello(&mut sink, &hello).await.is_err());
        assert!(sink.is_empty());

        // Counts and lengths inside a batch are checked against their limits.
        assert!(decode_batch(&[1, 1, 1, 0, 0, 0, 0]).is_err());
        assert!(decode_batch(&[0, 1, 5, 0x10, 0x01]).is_err());
        assert!(decode_batch(&[0, 1, 5, 0, 9, b'a']).is_err());

        // Picture streams: a record above its codec's limit fails before
        // its payload is allocated, on both sides.
        let mut head = [0u8; RECORD_HEADER_BYTES];
        head[..4].copy_from_slice(&(MAX_RECORD_BYTES as u32 + 1).to_be_bytes());
        let error = read_frame_record(&mut head.as_slice(), &video_header())
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        head[..4].copy_from_slice(&(MAX_TILES_RECORD_BYTES as u32 + 1).to_be_bytes());
        let error = read_frame_record(&mut head.as_slice(), &tiles_header())
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let oversize = record(1, FLAG_KEYFRAME, vec![0; MAX_TILES_RECORD_BYTES + 1]);
        assert!(write_frame_record(&mut sink, &tiles_header(), &oversize)
            .await
            .is_err());
        assert!(sink.is_empty());

        // Frame sizes: 4096 per side and 4096 × 2304 pixels in any orientation.
        let mut head = video_header().encode().unwrap();
        head[16..18].copy_from_slice(&4097u16.to_be_bytes());
        assert!(GopHeader::decode(&head).is_err());
        assert!(FrameSize::new(2304, 4096).is_valid_frame());
        assert!(TileDecoder::new(FrameSize::new(4096, 4096)).is_err());
        let mut decoder = TileDecoder::new(FrameSize::new(64, 64)).unwrap();
        assert!(decoder
            .apply_record(&vec![0; MAX_TILES_RECORD_BYTES + 1])
            .is_err());
        let lying = [0, 1, 0, 0, 0, 0, 64, 64, 1, 0xFF, 0xFF, 0xFF, 0xFF];
        assert!(decoder.apply_record(&lying).is_err());
    });
}
