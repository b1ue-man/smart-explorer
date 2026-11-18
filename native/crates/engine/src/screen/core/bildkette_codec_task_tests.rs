//! Acceptance of plan 2026-11-14 „Bildkette“ BK6 (B-30 to B-32, B-09's
//! `RATE_CEILING`): the rank of the pairings, the negotiation v2 against a
//! peer of v0.5.196 in both roles, that an old list never carries a codec
//! an old peer cannot read, and the control frame `RATE_CEILING` with and
//! without the host's announcement.
//!
//! A peer of v0.5.196 is modelled by the structures of tag `v0.5.196`
//! (`native/crates/engine/src/screen/core/wire_hello.rs`,
//! `negotiate.rs`): its `CodecKind` knows `h264` and `tiles` only and has no
//! `#[serde(other)]`; none of its hellos denies unknown fields.
use super::{
    rate_ceiling_negotiated, ScreenClientHello, ScreenHelloOk, ScreenServerHello, CODECS_V2,
};
use crate::screen::core::negotiate::{
    negotiate, sharp_alternative, CodecKind, CodecOffer, CodecPreference, H264Profile,
};
use crate::screen::core::wire_control::{decode_viewer_frame, encode_viewer_frame, ViewerFrame};

/// The structures of v0.5.196 that read codec lists and hellos.
mod v0_5_196 {
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum CodecKind {
        H264,
        Tiles,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum H264Profile {
        #[serde(rename = "cb")]
        ConstrainedBaseline,
        #[serde(rename = "high")]
        High,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct CodecOffer {
        pub codec: CodecKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub profile: Option<H264Profile>,
        #[serde(default)]
        pub hardware: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub pixel_access: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub max_macroblocks: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub max_macroblocks_per_second: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub max_width: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub max_height: Option<u32>,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ServerHello {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub frame_memory_class: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub family_version: Option<u16>,
        pub protocol_version: u16,
        pub challenge: String,
        pub server_device_id: String,
        pub server_public_key: String,
        pub server_fingerprint: String,
        pub server_node_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub icon_version: Option<u8>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub family_feed_version: Option<u16>,
    }

    /// The client hello; fields whose types did not change since stay JSON.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct ClientHello {
        pub protocol_version: u16,
        pub relation_kind: String,
        pub relation_id: String,
        pub device_id: String,
        pub device_name: String,
        pub public_key: String,
        pub fingerprint: String,
        pub node_id: String,
        pub client_nonce: String,
        pub proof: String,
        pub mode: serde_json::Value,
        #[serde(default)]
        pub source: serde_json::Value,
        #[serde(default)]
        pub decoders: Vec<CodecOffer>,
        #[serde(default)]
        pub display_size: serde_json::Value,
        pub max_fps: u16,
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct HelloOk {
        pub authorization: serde_json::Value,
        pub platform: serde_json::Value,
        #[serde(default)]
        pub encoders: Vec<CodecOffer>,
        pub approval: serde_json::Value,
    }
}

fn hw(codec: CodecKind) -> CodecOffer {
    CodecOffer::video(codec, false, true)
}

fn sw(codec: CodecKind) -> CodecOffer {
    CodecOffer::video(codec, false, false)
}

fn high(hardware: bool) -> CodecOffer {
    CodecOffer::h264(H264Profile::High, hardware)
}

type Row = (CodecKind, Option<H264Profile>, bool, bool);

fn rows(encoders: &[CodecOffer], decoders: &[CodecOffer]) -> Vec<Row> {
    negotiate(encoders, decoders, CodecPreference::Auto)
        .iter()
        .map(|c| (c.codec, c.profile, c.encoder_hardware, c.decoder_hardware))
        .collect()
}

const TILES: Row = (CodecKind::Tiles, None, false, false);
const HIGH: Option<H264Profile> = Some(H264Profile::High);

/// The bytes of a v0.5.196 server hello (`ScreenServerHello::new` of that
/// tag: no `cursor_overlay`, `codecs_v2`, `rate_ceiling`).
fn old_server_hello() -> ScreenServerHello {
    let old = v0_5_196::ServerHello {
        frame_memory_class: Some(2),
        family_version: Some(1),
        protocol_version: 1,
        challenge: "c".repeat(32),
        server_device_id: "host".into(),
        server_public_key: "key".into(),
        server_fingerprint: "pin".into(),
        server_node_id: "node".into(),
        icon_version: Some(1),
        family_feed_version: Some(1),
    };
    serde_json::from_slice(&serde_json::to_vec(&old).unwrap()).unwrap()
}

fn new_server_hello() -> ScreenServerHello {
    ScreenServerHello::new(
        "c".repeat(32),
        "host".into(),
        "key".into(),
        "pin".into(),
        "node".into(),
    )
}

/// The bytes of a v0.5.196 client hello (the shape of
/// `batches_session_budget_compatibility_task_tests.rs`) with H.264.
fn old_client_hello() -> ScreenClientHello {
    serde_json::from_value(
        serde_json::json!({"protocol_version":1,"relation_kind":"direct",
        "relation_id":"fixture","device_id":"viewer","device_name":"viewer","public_key":"key",
        "fingerprint":"pin","node_id":"node","client_nonce":"nonce","proof":"proof",
        "mode":"view","source":"synthetic",
        "decoders":[{"codec":"h264","profile":"high","hardware":true},{"codec":"tiles"}],
        "display_size":{"width":1920,"height":1080},"max_fps":60}),
    )
    .unwrap()
}

fn hello_ok() -> ScreenHelloOk {
    serde_json::from_value(
        serde_json::json!({"authorization":{"view":"allow","control":"allow"},
        "platform":"linux","approval":"not_required"}),
    )
    .unwrap()
}

fn host_encoders() -> Vec<CodecOffer> {
    vec![
        hw(CodecKind::Av1),
        hw(CodecKind::Hevc),
        high(true),
        sw(CodecKind::Av1),
        CodecOffer::tiles(),
    ]
}

/// B-30: hardware/hardware before hardware/software; within hardware/
/// hardware HEVC, H.264, AV1 (P9 2026-11-17: AV1 never first); otherwise H.264 before software AV1; 4:2:0
/// before 4:4:4; tiles last.
#[test]
fn bildkette_task_codec_rank_of_the_pairings() {
    let viewer = [hw(CodecKind::Av1), hw(CodecKind::Hevc), high(true)];
    assert_eq!(
        rows(&host_encoders(), &viewer),
        vec![
            (CodecKind::Hevc, None, true, true),
            (CodecKind::H264, HIGH, true, true),
            (CodecKind::Av1, None, true, true),
            (CodecKind::Av1, None, false, true),
            TILES,
        ]
    );
    // A software viewer: H.264 before HEVC before AV1 within the tier.
    let viewer = [sw(CodecKind::Av1), sw(CodecKind::Hevc), high(false)];
    assert_eq!(
        rows(&host_encoders(), &viewer),
        vec![
            (CodecKind::H264, HIGH, true, false),
            (CodecKind::Hevc, None, true, false),
            (CodecKind::Av1, None, true, false),
            (CodecKind::Av1, None, false, false),
            TILES,
        ]
    );
    // H.264 in hardware on both sides beats AV1 that a viewer decodes in
    // software (B-30: hardware/hardware before hardware/software).
    let viewer = [sw(CodecKind::Av1), high(true)];
    assert_eq!(
        rows(&host_encoders(), &viewer)[0],
        (CodecKind::H264, HIGH, true, true)
    );
    // Software on both sides: OpenH264 before SVT-AV1/dav1d.
    let host = [
        sw(CodecKind::Av1),
        CodecOffer::h264(H264Profile::ConstrainedBaseline, false),
    ];
    let viewer = [sw(CodecKind::Av1), high(false)];
    let order = rows(&host, &viewer);
    assert_eq!(order[0].0, CodecKind::H264);
    assert_eq!(order[1].0, CodecKind::Av1);
    assert_eq!(order.last(), Some(&TILES));
    // B-32: 4:4:4 only when both report it, after 4:2:0, and it is the
    // alternative for „Schärfe vor Flüssigkeit“.
    let host = [
        CodecOffer::video(CodecKind::Hevc, true, true),
        hw(CodecKind::Hevc),
    ];
    let choices = negotiate(
        &host,
        &[CodecOffer::video(CodecKind::Hevc, true, true)],
        CodecPreference::Auto,
    );
    assert_eq!(choices[0].profile, None);
    assert_eq!(choices[1].profile, HIGH);
    assert_eq!(sharp_alternative(&choices, &choices[0]), Some(choices[1]));
    let only_420 = rows(
        &[CodecOffer::video(CodecKind::Hevc, true, true)],
        &[hw(CodecKind::Hevc)],
    );
    assert_eq!(only_420, vec![TILES], "a 4:2:0 decoder never gets 4:4:4");
    // A named codec excludes the others and the tiles (`se screen bench`).
    let named = negotiate(
        &host_encoders(),
        &[hw(CodecKind::Hevc), high(true)],
        CodecPreference::Hevc,
    );
    assert!(named.iter().all(|choice| choice.codec == CodecKind::Hevc));
}

/// B-31, old viewer against a new host: no `decoders_v2`, so `HELLO_OK`
/// names H.264 and the tiles only, an old reader parses it, and the
/// session runs H.264.
#[test]
fn bildkette_task_codec_v2_old_viewer_new_host() {
    let viewer = old_client_hello();
    assert_eq!(viewer.decoders_v2, None);
    viewer.validate().unwrap();
    let mut ok = hello_ok();
    ok.offer_encoders(&host_encoders(), &viewer);
    ok.validate().unwrap();
    assert_eq!(ok.encoders_v2, None);
    assert!(ok.encoders.iter().all(|offer| offer.codec.is_legacy()));
    let json = serde_json::to_value(&ok).unwrap();
    assert!(json.get("encoders_v2").is_none());
    let old: v0_5_196::HelloOk = serde_json::from_value(json).unwrap();
    assert_eq!(old.encoders.len(), ok.encoders.len());
    let first = negotiate(
        &ok.all_encoders(),
        &viewer.all_decoders(),
        CodecPreference::Auto,
    )[0];
    assert_eq!(first.codec, CodecKind::H264);
}

/// B-31, new viewer against an old host: the old server hello lacks
/// `codecs_v2`, so the viewer names no HEVC or AV1 at all, an old host
/// parses its hello, and the session runs H.264.
#[test]
fn bildkette_task_codec_v2_new_viewer_old_host() {
    let server = old_server_hello();
    assert!(!server.codecs_v2());
    assert!(!rate_ceiling_negotiated(&server));
    let mut viewer = old_client_hello();
    let all = [
        hw(CodecKind::Av1),
        hw(CodecKind::Hevc),
        high(true),
        CodecOffer::tiles(),
    ];
    viewer.offer_decoders(&all, &server);
    assert_eq!(viewer.decoders_v2, None);
    let json = serde_json::to_value(&viewer).unwrap();
    assert!(json.get("decoders_v2").is_none());
    let old: v0_5_196::ClientHello = serde_json::from_value(json).unwrap();
    assert_eq!(
        old.decoders
            .iter()
            .map(|offer| offer.codec)
            .collect::<Vec<_>>(),
        vec![v0_5_196::CodecKind::H264, v0_5_196::CodecKind::Tiles]
    );
    // The old host's negotiation only sees what it can read.
    let first = negotiate(
        &[high(true), CodecOffer::tiles()],
        &viewer.all_decoders(),
        CodecPreference::Auto,
    )[0];
    assert_eq!(first.codec, CodecKind::H264);
}

/// B-31 between new peers, and an old reader of their hellos: it ignores
/// `codecs_v2`, `rate_ceiling`, `decoders_v2` and `encoders_v2`.
#[test]
fn bildkette_task_codec_v2_new_peers_and_an_old_reader() {
    let server = new_server_hello();
    assert_eq!(server.codecs_v2, Some(CODECS_V2));
    assert!(rate_ceiling_negotiated(&server));
    let old: v0_5_196::ServerHello =
        serde_json::from_value(serde_json::to_value(&server).unwrap()).unwrap();
    assert_eq!(old.server_node_id, "node");
    let mut viewer = old_client_hello();
    let all = [
        hw(CodecKind::Av1),
        hw(CodecKind::Hevc),
        high(true),
        CodecOffer::tiles(),
    ];
    viewer.offer_decoders(&all, &server);
    viewer.validate().unwrap();
    assert_eq!(
        viewer.decoders_v2,
        Some(vec![hw(CodecKind::Av1), hw(CodecKind::Hevc)])
    );
    let old: v0_5_196::ClientHello =
        serde_json::from_value(serde_json::to_value(&viewer).unwrap()).unwrap();
    assert_eq!(old.decoders.len(), 2);
    let mut ok = hello_ok();
    ok.offer_encoders(&host_encoders(), &viewer);
    ok.validate().unwrap();
    let v2 = ok.encoders_v2.clone().unwrap();
    assert!(v2.iter().all(|offer| !offer.codec.is_legacy()));
    let old: v0_5_196::HelloOk =
        serde_json::from_value(serde_json::to_value(&ok).unwrap()).unwrap();
    assert_eq!(old.encoders.len(), ok.encoders.len());
    let first = negotiate(
        &ok.all_encoders(),
        &viewer.all_decoders(),
        CodecPreference::Auto,
    )[0];
    assert_eq!(
        (first.codec, first.encoder_hardware, first.decoder_hardware),
        // AV1 is never first in Automatic (P9 „av1 ist zu viel“).
        (CodecKind::Hevc, true, true)
    );
    // A v2 list never repeats a legacy codec.
    viewer.decoders_v2 = Some(vec![high(true)]);
    assert!(viewer.validate().is_err());
}

/// B-31: an HEVC or AV1 entry never enters an old list, in either role,
/// whatever the platform lists; the picture stream codes are 3 and 4.
#[test]
fn bildkette_task_codec_unknown_codec_never_in_the_old_field() {
    let newer = [
        hw(CodecKind::Hevc),
        sw(CodecKind::Av1),
        CodecOffer::video(CodecKind::Av1, true, true),
    ];
    for server in [new_server_hello(), old_server_hello()] {
        let mut viewer = old_client_hello();
        viewer.offer_decoders(&newer, &server);
        let json = serde_json::to_value(&viewer).unwrap();
        let old: v0_5_196::ClientHello = serde_json::from_value(json).unwrap();
        assert!(old.decoders.is_empty());
        let mut ok = hello_ok();
        ok.offer_encoders(&newer, &viewer);
        let json = serde_json::to_value(&ok).unwrap();
        let old: v0_5_196::HelloOk = serde_json::from_value(json).unwrap();
        assert!(old.encoders.is_empty());
    }
    let text = serde_json::to_string(&old_client_hello()).unwrap();
    assert!(!text.contains("hevc") && !text.contains("av1"));
    assert_eq!(CodecKind::from_wire_code(3), Some(CodecKind::Av1));
    assert_eq!(CodecKind::from_wire_code(4), Some(CodecKind::Hevc));
    assert!(CodecKind::Hevc.profile_valid(None) && CodecKind::Av1.profile_valid(HIGH));
    assert!(!CodecKind::Hevc.profile_valid(Some(H264Profile::ConstrainedBaseline)));
    assert!(!CodecKind::H264.profile_valid(None));
}

/// B-09/BK6: `RATE_CEILING` (tag 50, 8 bytes, 0 = automatic) round-trips;
/// the viewer may send it only to a host with `rate_ceiling_v1`, because an
/// unknown viewer frame ends the session at an older host.
#[test]
fn bildkette_task_codec_rate_ceiling_with_and_without_the_capability() {
    for ceiling in [Some(20_000_000u64), None] {
        let (tag, payload) = encode_viewer_frame(&ViewerFrame::RateCeiling(ceiling)).unwrap();
        assert_eq!(tag, crate::screen::core::wire_control::RATE_CEILING);
        assert_eq!(payload.len(), 8);
        assert_eq!(
            decode_viewer_frame(tag, &payload).unwrap(),
            ViewerFrame::RateCeiling(ceiling)
        );
    }
    assert!(decode_viewer_frame(crate::screen::core::wire_control::RATE_CEILING, &[0; 7]).is_err());
    // v0.5.196 refuses a tag it does not know (`wire_control_codec.rs` of the
    // tag: "unknown screen frame from the viewer"); a tag unknown to this
    // build fails the same way.
    assert!(decode_viewer_frame(250, &[0; 8]).is_err());
    assert!(rate_ceiling_negotiated(&new_server_hello()));
    assert!(!rate_ceiling_negotiated(&old_server_hello()));
}

#[path = "bildkette_codec_register_task_tests.rs"]
mod register;
