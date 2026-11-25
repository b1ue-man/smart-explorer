use super::video_chain::{ChainGate, Verdict};
use super::video_route::{loss_recovery, LossRecovery};
use super::*;
use crate::screen::core::gop_continue::ResetFacts;
use crate::screen::core::wire_control::{decode_viewer_frame, encode_viewer_frame, ViewerFrame};

fn server() -> ScreenServerHello {
    ScreenServerHello::new(
        "c".repeat(32),
        "host".into(),
        "key".into(),
        "pin".into(),
        "node".into(),
    )
}

#[test]
fn udp_task_carriers_need_both_sides() {
    let new = server();
    assert_eq!(new.video_carriers, host_carriers());
    let both = Carriers {
        datagrams: true,
        udp: true,
    };
    assert_eq!(viewer_carriers(&new, SessionMode::View, false), both);
    assert_eq!(viewer_carriers(&new, SessionMode::Control, false), both);
    // A probe and a metadata channel offer nothing.
    assert!(!viewer_carriers(&new, SessionMode::Probe, false).any());
    assert!(!viewer_carriers(&new, SessionMode::View, true).any());
    // The direct TCP link: for a full session to a host that names it.
    assert!(viewer_tcp(&new, SessionMode::View, false));
    assert!(!viewer_tcp(&new, SessionMode::Probe, false));
    assert!(!viewer_tcp(&new, SessionMode::View, true));
    // A host of v0.5.204 names no carrier: its viewer keeps the streams.
    let mut json = serde_json::to_value(&new).unwrap();
    let fields = json.as_object_mut().unwrap();
    fields.remove("video_carriers");
    fields.remove("video_tcp");
    let old: ScreenServerHello = serde_json::from_value(json).unwrap();
    assert!(!viewer_carriers(&old, SessionMode::View, false).any());
    assert!(!viewer_tcp(&old, SessionMode::View, false));
    // A host that knows only the QUIC datagrams.
    let mut quic_only = new.clone();
    quic_only.video_carriers = vec![VIDEO_DATAGRAMS_V1.into()];
    let offered = viewer_carriers(&quic_only, SessionMode::View, false);
    assert_eq!(offered.names(), vec![VIDEO_DATAGRAMS_V1.to_string()]);
    // The host takes what both read; names of newer builds are ignored.
    let offer = VideoOffer {
        carriers: vec!["video_tcp_v1".into(), VIDEO_UDP_V1.into()],
        candidates: Vec::new(),
    };
    let taken = Carriers::of(&offer.carriers).and(both);
    assert_eq!((taken.udp, taken.datagrams), (true, false));
    assert!(!Carriers::of(&[]).any());
}

#[test]
fn udp_task_control_frames_round_trip_and_refuse_bad_payloads() {
    let loss = FrameLoss {
        gop_id: 9,
        frame_id: u64::MAX,
    };
    let (tag, payload) = encode_viewer_frame(&ViewerFrame::FrameLoss(loss)).unwrap();
    assert_eq!((tag, payload.len()), (FRAME_LOSS, FRAME_LOSS_BYTES));
    assert_eq!(
        decode_viewer_frame(tag, &payload).unwrap(),
        ViewerFrame::FrameLoss(loss)
    );
    assert!(decode_viewer_frame(FRAME_LOSS, &payload[..15]).is_err());
    assert!(decode_viewer_frame(FRAME_LOSS, &[0; 17]).is_err());
    let offer = VideoOffer {
        carriers: host_carriers(),
        candidates: vec!["192.0.2.1:4000".into(), "[2001:db8::1]:4000".into()],
    };
    let frame = ViewerFrame::VideoOffer(offer.clone());
    let (tag, payload) = encode_viewer_frame(&frame).unwrap();
    assert_eq!(tag, VIDEO_OFFER);
    assert_eq!(decode_viewer_frame(tag, &payload).unwrap(), frame);
    assert_eq!(offer.candidates().len(), 2);
    let too_many = VideoOffer {
        candidates: vec!["192.0.2.1:1".into(); MAX_CANDIDATES + 1],
        ..offer.clone()
    };
    assert!(encode_viewer_frame(&ViewerFrame::VideoOffer(too_many)).is_err());
    assert!(decode_viewer_frame(VIDEO_OFFER, br#"{"carriers":[],"candidates":["host"]}"#).is_err());
    assert!(decode_viewer_frame(VIDEO_OFFER, &vec![b' '; VIDEO_OFFER_BYTES + 1]).is_err());
    let report = VideoReport {
        pictures: 600,
        lost_pictures: 2,
        skipped_pictures: 3,
        recovered_pictures: 40,
        lost_packets: 90,
        recovered_packets: 85,
        late_packets: 1,
        direct_packets: 9_000,
    };
    let (tag, payload) = encode_viewer_frame(&ViewerFrame::VideoReport(report)).unwrap();
    assert_eq!(tag, VIDEO_REPORT);
    assert!(payload.len() <= VIDEO_REPORT_BYTES);
    assert_eq!(
        decode_viewer_frame(tag, &payload).unwrap(),
        ViewerFrame::VideoReport(report)
    );
    // The viewer's frames of the direct TCP link (tags 240, 242).
    use crate::screen::core::video_tcp_wire::{
        CandidateKind, TcpCandidate, TcpFailureReason, VideoTcpOffer, VideoTcpResult,
        VIDEO_TCP_OFFER, VIDEO_TCP_RESULT,
    };
    let tcp_offer = ViewerFrame::VideoTcpOffer(VideoTcpOffer {
        candidates: vec![TcpCandidate {
            addr: "192.0.2.7:40000".parse().unwrap(),
            kind: CandidateKind::Local,
        }],
    });
    let (tag, payload) = encode_viewer_frame(&tcp_offer).unwrap();
    assert_eq!(tag, VIDEO_TCP_OFFER);
    assert_eq!(decode_viewer_frame(tag, &payload).unwrap(), tcp_offer);
    let failed = ViewerFrame::VideoTcpResult(VideoTcpResult::Failed {
        reason: TcpFailureReason::Unreachable,
    });
    let (tag, payload) = encode_viewer_frame(&failed).unwrap();
    assert_eq!(tag, VIDEO_TCP_RESULT);
    assert_eq!(decode_viewer_frame(tag, &payload).unwrap(), failed);
    // The host's answer: the key in hex and back; a short key is refused.
    let key = [0xa5u8; 32];
    let answer = VideoAnswer::new(77, &key, &["198.51.100.2:9".parse().unwrap()]);
    answer.validate().unwrap();
    let json = serde_json::to_vec(&answer).unwrap();
    assert!(json.len() <= VIDEO_ANSWER_BYTES);
    let back: VideoAnswer = serde_json::from_slice(&json).unwrap();
    assert_eq!(back.key_bytes().unwrap(), key);
    assert_eq!(
        back.candidates(),
        vec!["198.51.100.2:9".parse::<SocketAddr>().unwrap()]
    );
    let short = VideoAnswer {
        key: "a5".repeat(31),
        ..back.clone()
    };
    assert!(short.validate().is_err());
    let not_hex = VideoAnswer {
        key: "zz".repeat(32),
        ..back
    };
    assert!(not_hex.validate().is_err());
}

#[test]
fn udp_task_chain_skips_until_recovery_and_reports_once() {
    let mut gate = ChainGate::default();
    // Group 1: key frame 1, pictures 2 and 3 linked.
    assert_eq!(gate.picture(1, 1, 0, true, false), Verdict::Deliver);
    assert_eq!(gate.picture(1, 2, 1, false, false), Verdict::Deliver);
    assert_eq!(gate.picture(1, 3, 2, false, false), Verdict::Deliver);
    // Picture 4 is given up: reported once, 5 and 6 are skipped.
    let loss = FrameLoss {
        gop_id: 1,
        frame_id: 4,
    };
    assert_eq!(gate.gave_up(1, 4), Some(loss));
    assert_eq!(gate.picture(1, 5, 4, false, false), Verdict::Skip);
    assert_eq!(gate.gave_up(1, 6), None);
    assert!(gate.broken(1));
    // The host continues in group 2 from picture 3: handed on again.
    assert_eq!(gate.picture(2, 7, 0, false, true), Verdict::Deliver);
    assert!(!gate.broken(2));
    assert_eq!(gate.picture(2, 8, 7, false, false), Verdict::Deliver);
    // No packet of picture 9 came: picture 10 names it and breaks the chain.
    assert_eq!(
        gate.picture(2, 10, 9, false, false),
        Verdict::Lost(FrameLoss {
            gop_id: 2,
            frame_id: 9
        })
    );
    assert_eq!(gate.picture(2, 11, 10, false, false), Verdict::Skip);
    // A key frame inside the broken group recovers it.
    assert_eq!(gate.picture(2, 12, 11, true, false), Verdict::Deliver);
    assert_eq!(gate.picture(2, 13, 12, false, false), Verdict::Deliver);
    // A group whose first picture never came: its second one breaks it; a
    // late picture of a replaced group is skipped without a report.
    assert_eq!(
        gate.picture(3, 15, 14, false, false),
        Verdict::Lost(FrameLoss {
            gop_id: 3,
            frame_id: 14
        })
    );
    assert_eq!(gate.picture(4, 16, 0, true, false), Verdict::Deliver);
    assert_eq!(gate.gave_up(3, 15), None);
    assert_eq!(gate.picture(3, 17, 15, false, false), Verdict::Skip);
    // A new group that starts with neither key frame nor continuation.
    assert!(matches!(
        gate.picture(5, 18, 0, false, false),
        Verdict::Lost(_)
    ));
}

#[test]
fn udp_task_host_recovers_a_loss_by_continuation_or_key_frame() {
    let continuable = ResetFacts {
        negotiated: true,
        video: true,
        keyframe_due: false,
        encoder_running: true,
        stage_continues: true,
        last_good: Some(41),
    };
    let loss = FrameLoss {
        gop_id: 6,
        frame_id: 43,
    };
    assert_eq!(
        loss_recovery(Some(6), loss, &continuable),
        LossRecovery::Continue(41)
    );
    // A loss of a group the host replaced already is answered.
    assert_eq!(
        loss_recovery(Some(7), loss, &continuable),
        LossRecovery::Ignore
    );
    assert_eq!(
        loss_recovery(None, loss, &continuable),
        LossRecovery::Ignore
    );
    // Without a confirmed picture, a continuing decoder, a running encoder
    // or with a key frame due: a key frame.
    let cases = [
        ResetFacts {
            last_good: None,
            ..continuable
        },
        ResetFacts {
            negotiated: false,
            ..continuable
        },
        ResetFacts {
            encoder_running: false,
            ..continuable
        },
        ResetFacts {
            stage_continues: false,
            ..continuable
        },
        ResetFacts {
            keyframe_due: true,
            ..continuable
        },
    ];
    for facts in cases {
        assert_eq!(
            loss_recovery(Some(6), loss, &facts),
            LossRecovery::KeyFrame,
            "{facts:?}"
        );
    }
}
