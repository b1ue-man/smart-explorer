//! Tests of the protocol rests of the picture chain (plan 2026-11-14
//! „Bildkette“ P1): the continuation per codec in the negotiation,
//! `SET_DECODERS` with and without the host's capability, and old peers in
//! both roles. The pipeline's reaction to a new pairing is tested in
//! `bildkette_proto_pairing_task_tests.rs`.
use super::*;
use crate::screen::core::gop_continue::{ResetFacts, GOP_CONTINUE_V1};
use crate::screen::core::negotiate::{negotiate, H264Profile};
use crate::screen::core::wire_control::{decode_viewer_frame, encode_viewer_frame, ViewerFrame};
use crate::screen::core::wire_hello::ScreenClientHello;

fn hevc(hardware: bool) -> CodecOffer {
    CodecOffer::video(CodecKind::Hevc, false, hardware)
}

fn av1(hardware: bool) -> CodecOffer {
    CodecOffer::video(CodecKind::Av1, false, hardware)
}

fn h264(hardware: bool) -> CodecOffer {
    CodecOffer::h264(H264Profile::High, hardware)
}

/// The bytes of a client hello of v0.5.196 (`bildkette_codec_task_tests`).
fn old_client_json() -> serde_json::Value {
    serde_json::json!({"protocol_version":1,"relation_kind":"direct",
        "relation_id":"fixture","device_id":"viewer","device_name":"viewer","public_key":"key",
        "fingerprint":"pin","node_id":"node","client_nonce":"nonce","proof":"proof",
        "mode":"view","source":"synthetic",
        "decoders":[{"codec":"h264","profile":"high","hardware":true},{"codec":"tiles"}],
        "display_size":{"width":1920,"height":1080},"max_fps":60})
}

fn client(extra: serde_json::Value) -> ScreenClientHello {
    let mut json = old_client_json();
    for (key, value) in extra.as_object().unwrap() {
        json[key] = value.clone();
    }
    let hello: ScreenClientHello = serde_json::from_value(json).unwrap();
    hello.validate().unwrap();
    hello
}

/// The pairing a host of this version takes for a Windows viewer with
/// H.264, HEVC and AV1 decoders under „Codec: Automatisch“.
fn pairing(encoders: &[CodecOffer], decoders: &[CodecOffer]) -> CodecChoice {
    negotiate(encoders, decoders, CodecPreference::Auto)[0]
}

/// B-36 per codec: the host continues when the negotiated decoder is one of
/// `gop_continue_decoders`; a hello without the field behaves as W3 (the
/// whole-session `gop_continue`) and a hello of v0.5.196 never continues.
#[test]
fn bildkette_task_proto_continue_per_codec_in_the_negotiation() {
    let viewer = [h264(true), hevc(true), av1(true), CodecOffer::tiles()];
    // „Automatisch“ on Windows: H.264 cannot continue, HEVC and AV1 can;
    // the whole-session field stays off, the list names HEVC and AV1.
    let hello = client(serde_json::json!({
        "gop_continue_decoders": [hevc(true), av1(true)],
    }));
    assert_eq!(hello.gop_continue, None);
    let offers = ContinueOffers::from_hello(&hello);
    // P9: AV1 never ranks first in automatic, so a host with AV1 alone.
    let av1_host = [av1(true), CodecOffer::tiles()];
    let chosen = pairing(&av1_host, &viewer);
    assert_eq!(chosen.codec, CodecKind::Av1);
    assert!(offers.continues(&chosen));
    let h264_host = [h264(true), CodecOffer::tiles()];
    let chosen = pairing(&h264_host, &viewer);
    assert_eq!(chosen.codec, CodecKind::H264);
    assert!(!offers.continues(&chosen));
    // The stage counts: a hardware HEVC decoder says nothing of software.
    let soft = CodecChoice {
        decoder_hardware: false,
        ..pairing(&[hevc(true)], &[hevc(false)])
    };
    assert!(!offers.continues(&soft));
    // A 4:4:4 decoder that continues also decodes the 4:2:0 pairing.
    let sharp = ContinueOffers::new(false, vec![CodecOffer::video(CodecKind::Hevc, true, true)]);
    assert!(sharp.continues(&pairing(&[hevc(true)], &[hevc(true)])));
    // The tiles never continue, whatever is announced.
    let tiles = pairing(&[CodecOffer::tiles()], &[CodecOffer::tiles()]);
    assert!(!ContinueOffers::new(true, Vec::new()).continues(&tiles));
    // The host's decision after a stale stream (`ResetFacts`).
    let facts = |negotiated| ResetFacts {
        negotiated,
        video: true,
        keyframe_due: false,
        encoder_running: true,
        stage_continues: true,
        last_good: Some(41),
    };
    let av1_pairing = pairing(&av1_host, &viewer);
    assert_eq!(
        facts(offers.continues(&av1_pairing)).continue_from(),
        Some(41)
    );
    let h264_pairing = pairing(&h264_host, &viewer);
    assert_eq!(facts(offers.continues(&h264_pairing)).continue_from(), None);
    // A viewer of this batch before the field (W3): the whole session.
    let w3 = client(serde_json::json!({ "gop_continue": GOP_CONTINUE_V1 }));
    assert!(ContinueOffers::from_hello(&w3).continues(&h264_pairing));
    // A viewer of v0.5.196 names neither: never.
    let old = client(serde_json::json!({}));
    assert_eq!(old.gop_continue_decoders, None);
    assert!(!ContinueOffers::from_hello(&old).continues(&av1_pairing));
    // Without a list nothing new is written: the hello stays as before.
    let written = serde_json::to_value(&old).unwrap();
    assert!(written.get("gop_continue_decoders").is_none());
    // The list names video decoders only, at most 64.
    let mut bad = old.clone();
    bad.gop_continue_decoders = Some(vec![CodecOffer::tiles()]);
    assert!(bad.validate().is_err());
    bad.gop_continue_decoders = Some(vec![hevc(true); MAX_COMBINED_OFFERS + 1]);
    assert!(bad.validate().is_err());
}

/// A reader of v0.5.196: its `CodecKind` knows H.264 and the tiles only and
/// has no `#[serde(other)]`, and none of its hellos denies unknown fields.
mod v0_5_196 {
    #![allow(dead_code)]
    use serde::Deserialize;

    #[derive(Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum CodecKind {
        H264,
        Tiles,
    }

    #[derive(Deserialize)]
    pub struct CodecOffer {
        pub codec: CodecKind,
    }

    #[derive(Deserialize)]
    pub struct ClientHello {
        pub protocol_version: u16,
        #[serde(default)]
        pub decoders: Vec<CodecOffer>,
    }

    #[derive(Deserialize)]
    pub struct ServerHello {
        pub protocol_version: u16,
        pub challenge: String,
    }
}

/// Old peers in both roles: a reader of v0.5.196 reads a new client hello
/// with HEVC and AV1 in `gop_continue_decoders` (serde skips an unknown
/// field without reading its codecs) and a new server hello with
/// `set_decoders`; a new reader of an old server hello finds no capability.
#[test]
fn bildkette_task_proto_old_peers_in_both_roles() {
    let hello = client(serde_json::json!({
        "gop_continue_decoders": [hevc(true), av1(false)],
    }));
    let bytes = serde_json::to_vec(&hello).unwrap();
    let old: v0_5_196::ClientHello = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(old.protocol_version, 1);
    assert_eq!(old.decoders.len(), 2);
    let server = ScreenServerHello::new(
        "c".repeat(32),
        "host".into(),
        "key".into(),
        "pin".into(),
        "node".into(),
    );
    assert_eq!(server.set_decoders, Some(SET_DECODERS_V1));
    assert!(negotiated(&server));
    let bytes = serde_json::to_vec(&server).unwrap();
    let old: v0_5_196::ServerHello = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(old.challenge, "c".repeat(32));
    // A host of v0.5.196, or of this batch before `SET_DECODERS`.
    let mut json = serde_json::to_value(&server).unwrap();
    json.as_object_mut().unwrap().remove("set_decoders");
    let before: ScreenServerHello = serde_json::from_value(json).unwrap();
    assert_eq!(before.set_decoders, None);
    assert!(!negotiated(&before));
    let mut later = server.clone();
    later.set_decoders = Some(SET_DECODERS_V1 + 1);
    assert!(!negotiated(&later));
}

/// `SET_DECODERS`: tag 51 after `RATE_CEILING` (50), JSON, a round trip,
/// bounds; the capability decides whether the frame may leave the viewer.
/// A host without it ends the session on an unknown frame, as this
/// version's reader does with a tag it does not know.
#[test]
fn bildkette_task_proto_set_decoders_with_and_without_the_capability() {
    assert_eq!(
        SET_DECODERS,
        crate::screen::core::wire_control::RATE_CEILING + 1
    );
    let set = SetDecoders {
        decoders: vec![hevc(true), h264(true), CodecOffer::tiles()],
        gop_continue: false,
        gop_continue_decoders: vec![hevc(true)],
        codec_choice: None,
    };
    let (tag, payload) = encode_viewer_frame(&ViewerFrame::SetDecoders(set.clone())).unwrap();
    assert_eq!(tag, SET_DECODERS);
    assert_eq!(
        decode_viewer_frame(tag, &payload).unwrap(),
        ViewerFrame::SetDecoders(set.clone())
    );
    // Fields without a value are not written; an empty frame is read.
    let plain = SetDecoders {
        decoders: vec![CodecOffer::tiles()],
        ..SetDecoders::default()
    };
    let json: serde_json::Value = serde_json::from_slice(
        &encode_viewer_frame(&ViewerFrame::SetDecoders(plain.clone()))
            .unwrap()
            .1,
    )
    .unwrap();
    assert!(json.get("gop_continue").is_none() && json.get("gop_continue_decoders").is_none());
    assert_eq!(
        decode_viewer_frame(SET_DECODERS, b"{}").unwrap(),
        ViewerFrame::SetDecoders(SetDecoders::default())
    );
    // Bounds on both sides.
    let many = SetDecoders {
        decoders: vec![h264(true); MAX_COMBINED_OFFERS + 1],
        ..SetDecoders::default()
    };
    assert!(encode_viewer_frame(&ViewerFrame::SetDecoders(many.clone())).is_err());
    assert!(decode_viewer_frame(SET_DECODERS, &serde_json::to_vec(&many).unwrap()).is_err());
    let tiles = SetDecoders {
        gop_continue_decoders: vec![CodecOffer::tiles()],
        ..SetDecoders::default()
    };
    assert!(tiles.validate().is_err());
    // The tag after `FRAME_TIMING` is unknown here, as 51 is to an older host.
    assert!(
        decode_viewer_frame(crate::screen::core::frame_timing::FRAME_TIMING + 1, b"{}").is_err()
    );
    // The continuation of the frame replaces the one of the hello.
    let offers = set.continuation();
    assert!(offers.continues(&pairing(&[hevc(true)], &[hevc(true)])));
    assert!(!offers.continues(&pairing(&[h264(true)], &[h264(true)])));
    // The frame a client builds from its request: the offers under its
    // „Codec“ (HEVC and the tiles) and its announcement.
    let request: crate::daemon::StreamRequest = serde_json::from_value(serde_json::json!({
        "mode": "view", "max_fps": 60, "codec": "hevc",
        "decoders": [h264(true), hevc(true), av1(true), CodecOffer::tiles()],
        "gop_continue_decoders": [hevc(true)],
    }))
    .unwrap();
    let frame = request.decoders_frame();
    assert_eq!(frame.decoders, vec![hevc(true)]);
    assert_eq!(frame.gop_continue_decoders, vec![hevc(true)]);
    // Review 2026-11-19 (Codex 3/6): the frame names the „Codec“ as well; a
    // „Scharf (GPU)“ viewer with H.264 alone reads as „Scharf (GPU)“ (GPU
    // encoders only), without the field as H.264, as before.
    assert_eq!(frame.codec_choice, Some(CodecSetting::Hevc));
    assert_eq!(
        preference_of(Some(CodecSetting::Sharp), &[h264(true)]),
        CodecPreference::Sharp
    );
    assert_eq!(preference_of(None, &[h264(true)]), CodecPreference::H264);
    assert_eq!(
        preference_of(Some(CodecSetting::Lossless), &[CodecOffer::tiles()]),
        CodecPreference::Auto
    );
    let soft = [h264(false), CodecOffer::tiles()];
    let sharp = SetDecoders {
        decoders: vec![h264(true)],
        codec_choice: Some(CodecSetting::Sharp),
        ..SetDecoders::default()
    };
    let refused =
        renegotiate_set(&soft, &sharp, None, true, CodecTrust::ALL).expect_err("no GPU encoder");
    assert_eq!(refused.preference, CodecPreference::Sharp);
    let read = serde_json::to_value(&sharp).unwrap();
    assert_eq!(read["codec_choice"], "sharp");
    let plain = serde_json::to_value(SetDecoders::default()).unwrap();
    assert!(plain.get("codec_choice").is_none());
    // A request of an older client (no field) still reads.
    let old: crate::daemon::StreamRequest =
        serde_json::from_value(serde_json::json!({"mode":"view","decoders":[],"max_fps":60}))
            .unwrap();
    assert!(old.gop_continue_decoders.is_empty());
    let back = serde_json::to_value(&old).unwrap();
    assert!(back.get("gop_continue_decoders").is_none());
}

/// The host's new negotiation uses the ranking of the opening; a session
/// that opened with the tiles keeps them; no pairing is `None`.
#[test]
fn bildkette_task_proto_renegotiation_follows_the_opening_rank() {
    let host = [av1(true), hevc(true), h264(true), CodecOffer::tiles()];
    let first = renegotiate(&host, &[h264(true), CodecOffer::tiles()], None, true).unwrap();
    assert_eq!(first.choice.codec, CodecKind::H264);
    let next = renegotiate(
        &host,
        &[hevc(true), h264(true), CodecOffer::tiles()],
        None,
        true,
    )
    .unwrap();
    assert_eq!(next.choice.codec, CodecKind::Hevc);
    assert_eq!(
        next.choice,
        negotiate(
            &host,
            &[hevc(true), h264(true), CodecOffer::tiles()],
            CodecPreference::Auto
        )[0]
    );
    assert!(next.tile_limit.is_some());
    // A software decoder stage: H.264 before HEVC outside hardware/hardware.
    let soft = renegotiate(&host, &[hevc(false), h264(false)], None, true).unwrap();
    assert_eq!(soft.choice.codec, CodecKind::H264);
    assert!(!soft.choice.decoder_hardware);
    // A tiles session stays with the tiles.
    let tiles = renegotiate(&host, &[hevc(true), CodecOffer::tiles()], None, false).unwrap();
    assert_eq!(tiles.choice.codec, CodecKind::Tiles);
    // Review 2026-11-19 (Codex 11): a chosen video codec in a session opened
    // for the tiles is refused with its reason, never ignored.
    let refused = renegotiate_for(&host, &[h264(true)], None, false, CodecTrust::ALL)
        .expect_err("tiles session");
    assert_eq!(refused.reason, TILES_SESSION);
    assert!(refused
        .to_string()
        .starts_with("chosen_codec_unavailable: h264: "));
    // The tiles are every pair's last fallback under Auto; a viewer that
    // names no decoder at all chose a codec it cannot decode (review
    // 2026-11-19): no tiles, the chosen codec's reason.
    assert!(renegotiate(&[], &[CodecOffer::tiles()], None, true).is_some());
    assert!(renegotiate(&[], &[], None, true).is_none());
    let refused = renegotiate_for(&host, &[], None, true, CodecTrust::ALL).expect_err("no offer");
    assert_eq!(
        refused.to_string(),
        "chosen_codec_unavailable: the viewer has no decoder of this codec"
    );
}
