//! The requests and replies inside `IpcRequest::Screen` and
//! `IpcResponse::Screen` (SCR-M2, strand S4): probe and measurement of
//! another device's screen, the relayed viewer session of the desktop
//! viewer (`Stream`), and the sessions this host runs (list, stop, wait for
//! a change). Answers to confirmations go through `IpcRequest::Access` of S1
//! (`Decide`), never through here. A daemon of 0.5.182 refuses the variant as
//! unknown, so a newer client never mistakes it for an answer.
//!
//! After `StreamOpened` the connection carries binary frames: tag `u8`,
//! length `u32` big-endian, payload. Worker to client: `RELAY_HELLO` (the
//! reply of the host's handshake as JSON, also `pending`), `RELAY_HOST` (a
//! host control frame: its tag and payload as on the screen wire),
//! `RELAY_RECORD` (a picture record: the 20-byte head of its picture group,
//! the optional length-prefixed flattened v2 contract/family-revision JSON,
//! the 21-byte record head, the payload), `RELAY_BROKEN` (a picture group
//! broke off: its id `u64`), `RELAY_CLOCK` (host clock minus viewer clock in
//! µs, `i64`), `RELAY_END` (JSON `{code, detail}`). Client to worker:
//! `RELAY_VIEWER` (a viewer control frame: tag and payload; the worker
//! acknowledges pictures and sends `PING` itself) and `RELAY_CLOSE`.
use serde::{Deserialize, Serialize};

use crate::screen::core::bench::BenchReport;
use crate::screen::core::input_model::InputCapabilities;
use crate::screen::core::negotiate::{CodecOffer, CodecSetting, DecoderPreference};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::registry::SessionView;
use crate::screen::core::window_list::{MonitorInfo, SourceId};
use crate::screen::core::wire_hello::{HostPlatform, SessionMode};
use crate::share::access_grant::{Capability, CapabilityDecision};
use crate::share::PeerOpenTarget;

#[path = "ipc_protocol_screen_bench.rs"]
mod bench;
pub use bench::{bench_source, monitor_missing, BenchRequest, BENCH_SOURCE_NOT_FOUND};
#[path = "ipc_protocol_screen_relay.rs"]
mod relay;
#[path = "ipc_protocol_screen_switch.rs"]
mod switch;
pub use relay::{
    decode_relay_record, encode_relay_record, read_relay, read_relay_budgeted, write_relay,
};
pub use switch::{RateCeilingState, RELAY_AUDIO, RELAY_RATE_CEILING};

/// Longest a `HostWatch` waits for a change before it answers anyway.
pub const WATCH_WAIT_SECS: u64 = 25;

/// What a viewer session asks the host for (the client hello's fields).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamRequest {
    /// Authenticated node of an app-start receipt; never inferred from a fresh target lookup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_node: Option<String>,
    pub mode: SessionMode,
    #[serde(default)]
    pub source: Option<SourceId>,
    #[serde(default)]
    pub decoders: Vec<CodecOffer>,
    #[serde(default)]
    pub display_size: Option<FrameSize>,
    #[serde(default)]
    pub frame_capabilities: Option<crate::screen::core::frame_budget::FrameCapabilities>,
    pub max_fps: u16,
    /// „Höchste Datenrate“ in bit/s (plan 2026-11-14 „Bildkette“ B-09);
    /// absent is automatic. A daemon of v0.5.196 ignores the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rate_bps: Option<u64>,
    /// „Codec“ of the viewer (plan 2026-11-14 „Bildkette“ 4.10, BK6): the
    /// worker offers the named codec and the tiles only; absent is
    /// automatic. „Verlustfrei“ travels as `SET_MODE` `lossless`, as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<CodecSetting>,
    /// „Dekodierer“ of the viewer (4.10): the worker offers only decoder
    /// stages of that kind; absent is automatic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decoder: Option<DecoderPreference>,
    /// B-36: the client keeps its decoder over a continuation
    /// (`screen::core::gop_continue`); the daemon announces `gop_continue`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gop_continue: bool,
    /// B-36 per codec: the client's decoders that keep their references
    /// over a continuation (`gop_continue_decoders` of the hello).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gop_continue_decoders: Vec<CodecOffer>,
    /// „Monitor“ and „Auflösung“ of the viewer (plan 2026-11-17 P5): se's
    /// own display at the host; absent asks for none (a daemon of v0.5.197
    /// ignores it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_display: Option<crate::screen::core::virtual_display::VirtualDisplayChoice>,
}

impl StreamRequest {
    /// The decoders the hello names under „Codec“ and „Dekodierer“: tiles
    /// under „Automatisch“ (a chosen codec offers none, the host reads the
    /// choice from that), tiles alone for „Verlustfrei“ (fix 2026-11-19), no
    /// tiles (software) beside video for a chosen „Hardware“ (Codex 4/15).
    pub fn chosen_decoders(&self) -> Vec<CodecOffer> {
        let setting = self.codec.unwrap_or_default();
        let decoder = self.decoder.unwrap_or_default();
        let codec = match setting {
            CodecSetting::Lossless => crate::screen::core::negotiate::CodecPreference::Tiles,
            setting => setting.preference(),
        };
        let mut offers = decoder.filter_offers(&codec.filter_decoders(&self.decoders));
        let hardware = decoder == DecoderPreference::Hardware && setting != CodecSetting::Lossless;
        offers.retain(|offer| !hardware || offer.hardware);
        offers
    }

    /// A named „Codec“/„Dekodierer“ that leaves no decoder to offer (fix
    /// 2026-11-19; empty offers read like „Automatisch“ at the host): the
    /// client refuses to open with this text.
    pub fn unavailable_choice(&self) -> Option<String> {
        let codec = self.codec.unwrap_or_default();
        let decoder = self.decoder.unwrap_or_default();
        let auto = codec.preference() == crate::screen::core::negotiate::CodecPreference::Auto;
        let named = !auto || decoder != DecoderPreference::Automatic;
        let (codec, decoder) = (codec.label(), decoder.label());
        (named && self.chosen_decoders().is_empty()).then(|| {
            format!("Codec „{codec}“ mit Dekodierer „{decoder}“: kein passender Dekodierer auf diesem Gerät; die Sitzung wird nicht geöffnet.")
        })
    }
}

/// One request of the screen strand.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScreenIpcRequest {
    /// Rights, platform, codecs, monitors and limits of `target`'s screen.
    Probe { target: PeerOpenTarget },
    /// List/icons without opening the host's capture source.
    Metadata { target: PeerOpenTarget },
    /// An old daemon must reject the variant instead of silently ignoring the pin.
    PinnedMetadata {
        target: PeerOpenTarget,
        expected_node: String,
    },
    /// A viewer session relayed over this connection.
    Stream {
        target: PeerOpenTarget,
        request: StreamRequest,
    },
    /// Requires a nonempty expected_node in request before opening the stream.
    PinnedStream {
        target: PeerOpenTarget,
        request: StreamRequest,
    },
    /// A measurement with the test picture of `target`.
    Bench {
        target: PeerOpenTarget,
        options: BenchRequest,
    },
    /// The sessions this host runs.
    HostSessions,
    /// Ends the session `session` of this host, or every one without it.
    HostStop {
        #[serde(default)]
        session: Option<String>,
    },
    /// The sessions once their revision passed `since` (at most
    /// `WATCH_WAIT_SECS` later).
    HostWatch { since: u64 },
}

/// `se screen probe --json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeReport {
    /// How this device names the other one.
    pub device: String,
    /// Whether it knows the screen protocol of this version.
    pub supported: bool,
    pub platform: Option<HostPlatform>,
    pub view: CapabilityDecision,
    pub control: CapabilityDecision,
    pub monitors: Vec<MonitorInfo>,
    pub input: InputCapabilities,
    pub codecs: Vec<CodecOffer>,
    pub limits: Vec<String>,
    /// What a viewer that asks for se's own display would get (plan
    /// 2026-11-17 P5); absent from a host before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtual_display: Option<crate::screen::core::virtual_display::VirtualDisplayReport>,
}

/// The replies of `ScreenIpcRequest`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScreenIpcReply {
    Probe(ProbeReport),
    /// The host asks its user; the final reply follows on this connection.
    Pending {
        capability: Capability,
    },
    Bench(BenchReport),
    /// The viewer session runs: relay frames follow.
    StreamOpened,
    Sessions {
        revision: u64,
        sessions: Vec<SessionView>,
    },
    Stopped {
        count: usize,
    },
}

// Relay frames of `Stream`, worker to client.
pub const RELAY_HELLO: u8 = 1;
pub const RELAY_HOST: u8 = 2;
pub const RELAY_RECORD: u8 = 3;
pub const RELAY_BROKEN: u8 = 4;
pub const RELAY_CLOCK: u8 = 5;
pub const RELAY_END: u8 = 6;
// Client to worker.
pub const RELAY_VIEWER: u8 = 16;
pub const RELAY_CLOSE: u8 = 17;

/// Largest relay payload: a picture record (16 MiB) with both heads.
pub const MAX_RELAY_BYTES: usize = 16 * 1024 * 1024 + 64 + 1026;
/// Largest relay payload from the client: one viewer control frame.
pub const MAX_RELAY_VIEWER_BYTES: usize = 64 * 1024 + 1;

/// The end of a relayed session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayEnd {
    pub code: String,
    #[serde(default)]
    pub detail: Option<String>,
}

/// The reply of the host's handshake as the relay forwards it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayHello {
    Pending {
        capability: Capability,
    },
    Accepted {
        hello: crate::screen::core::wire_hello::ScreenHelloOk,
        /// `direct` or `relay`.
        path: String,
        /// Private daemon allocation evidence; absent from the peer HELLO_OK.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        local_limits: Option<crate::screen::core::frame_budget::FrameLimits>,
        /// „Höchste Datenrate“ of this session (`StreamRequest`).
        #[serde(default)]
        rate_ceiling: RateCeilingState,
        /// The host announced `set_decoders_v1` (P1): a new „Codec“ or
        /// „Dekodierer“ goes out as `SET_DECODERS` instead of a new session.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        set_decoders: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::core::wire_binary::{
        FrameRecord, GopHeader, GOP_HEADER_BYTES, RECORD_HEADER_BYTES,
    };

    #[test]
    fn batches_task_b6_session_budget_private_relay_keeps_v1_and_rejects_oversize_before_read() {
        use crate::screen::core::{frame_budget::legacy_limits, frame_size_caps::PacketBudget};
        let local = legacy_limits(24 * 1024 * 1024, None).unwrap();
        let hello = serde_json::from_value(
            serde_json::json!({"authorization":{"view":"allow","control":"allow"},
            "platform":"windows","approval":"not_required"}),
        )
        .unwrap();
        let accepted = RelayHello::Accepted {
            hello,
            path: "direct".into(),
            local_limits: Some(local),
            rate_ceiling: RateCeilingState::Automatic,
            set_decoders: false,
        };
        let bytes = serde_json::to_vec(&accepted).unwrap();
        let accepted: RelayHello = serde_json::from_slice(&bytes).unwrap();
        let RelayHello::Accepted {
            hello,
            local_limits,
            ..
        } = accepted
        else {
            panic!("not Accepted")
        };
        assert!(hello.frame_contract.is_none());
        assert_eq!(local_limits, Some(local));
        let packet = PacketBudget::for_receive(hello.frame_contract, local_limits).unwrap();
        let bound = packet.record_bytes + GOP_HEADER_BYTES + RECORD_HEADER_BYTES;
        let mut bytes = vec![RELAY_RECORD];
        bytes.extend_from_slice(&((bound + 1) as u32).to_be_bytes());
        let error = read_relay_budgeted(&mut bytes.as_slice(), usize::MAX, bound).unwrap_err();
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::InvalidData,
            "no payload was supplied"
        );
    }

    /// 4.10 (BK6): „Codec“ and „Dekodierer“ travel in `StreamRequest` and
    /// narrow the offers; the relay says what became of the data rate.
    #[test]
    fn bildkette_task_codec_stream_request_and_ceiling_state() {
        use crate::screen::core::negotiate::{CodecKind, H264Profile};
        // A request of v0.5.196 without `codec`/`decoder`: automatic, and
        // automatic writes no field.
        let old = serde_json::json!({"mode":"view","decoders":[],"max_fps":60});
        let mut request: StreamRequest = serde_json::from_value(old).unwrap();
        assert_eq!((request.codec, request.decoder), (None, None));
        let back = serde_json::to_value(&request).unwrap();
        assert!(back.get("codec").is_none() && back.get("decoder").is_none());
        request.decoders = vec![
            CodecOffer::video(CodecKind::Av1, false, true),
            CodecOffer::video(CodecKind::Av1, false, false),
            CodecOffer::h264(H264Profile::High, true),
            CodecOffer::tiles(),
        ];
        assert_eq!(request.chosen_decoders(), request.decoders);
        request.codec = Some(CodecSetting::Av1);
        request.decoder = Some(DecoderPreference::Software);
        let chosen = request.chosen_decoders();
        // A chosen codec offers no tiles (plan 2026-11-18 „Netzrate“).
        assert_eq!(
            chosen,
            vec![CodecOffer::video(CodecKind::Av1, false, false)]
        );
        // „Verlustfrei“ offers the tiles alone (fix 2026-11-19).
        request.codec = Some(CodecSetting::Lossless);
        request.decoder = None;
        assert_eq!(request.chosen_decoders(), vec![CodecOffer::tiles()]);
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["codec"], "lossless");
        assert_eq!(
            serde_json::from_value::<StreamRequest>(json).unwrap(),
            request
        );
        // What became of „Höchste Datenrate“: said, not silent.
        assert_eq!(
            RateCeilingState::of(None, false),
            RateCeilingState::Automatic
        );
        assert_eq!(
            RateCeilingState::of(Some(8_000_000), true),
            RateCeilingState::Applied
        );
        assert_eq!(
            RateCeilingState::of(Some(8_000_000), false),
            RateCeilingState::UnsupportedByHost
        );
        let hello = serde_json::json!({"kind":"accepted","hello":{"authorization":
            {"view":"allow","control":"allow"},"platform":"linux","approval":"not_required"},
            "path":"direct"});
        let RelayHello::Accepted { rate_ceiling, .. } = serde_json::from_value(hello).unwrap()
        else {
            panic!("not Accepted");
        };
        assert_eq!(rate_ceiling, RateCeilingState::Automatic);
    }

    /// B-09: „Höchste Datenrate“ travels in `StreamRequest`; a request of
    /// v0.5.196 without it means automatic, and automatic adds no field.
    #[test]
    fn bildkette_task_rate_stream_request_carries_the_max_rate() {
        let old = serde_json::json!({"mode":"view","decoders":[],"max_fps":60});
        let request: StreamRequest = serde_json::from_value(old).unwrap();
        assert_eq!(request.max_rate_bps, None);
        let back = serde_json::to_value(&request).unwrap();
        assert!(back.get("max_rate_bps").is_none());
        let limited = StreamRequest {
            max_rate_bps: Some(20_000_000),
            ..request
        };
        let json = serde_json::to_value(&limited).unwrap();
        assert_eq!(json["max_rate_bps"], 20_000_000);
        assert_eq!(
            serde_json::from_value::<StreamRequest>(json).unwrap(),
            limited
        );
    }

    #[test]
    fn batches_task_b6_extended_4k_ipc_contract_is_complete() {
        use crate::screen::core::frame_budget::{FrameCapabilities, FrameContract};
        let caps =
            FrameCapabilities::admitted(FrameSize::new(16384, 16384), 768 * 1024 * 1024).unwrap();
        let contract = FrameContract::negotiate(Some(caps), Some(caps)).unwrap();
        let header = GopHeader {
            gop_id: 1,
            frame_contract: contract,
            family_layout_revision: None,
            kind: crate::screen::core::wire_binary::StreamKind::Video,
            codec: crate::screen::core::negotiate::CodecKind::Tiles,
            profile: None,
            size: FrameSize::new(3840, 2160),
        };
        let record = FrameRecord {
            frame_id: 1,
            capture_ts_us: 100,
            flags: crate::screen::core::wire_binary::FLAG_KEYFRAME,
            payload: vec![0],
        };
        let bytes = encode_relay_record(&header, &record).unwrap();
        assert_eq!(decode_relay_record(&bytes).unwrap(), (header, record));
    }

    #[test]
    fn screen_task_ipc_screen_wire_shapes() {
        let target = PeerOpenTarget::Direct {
            contact_id: "contact".into(),
        };
        let request = ScreenIpcRequest::Bench {
            target,
            options: BenchRequest {
                codec: "tiles".into(),
                size: FrameSize::new(1280, 720),
                fps: 30,
                seconds: 10,
                source: None,
            },
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["kind"], "bench");
        assert_eq!(json["options"]["size"]["width"], 1280);
        let back: ScreenIpcRequest = serde_json::from_value(json).unwrap();
        assert_eq!(back, request);
        let stop: ScreenIpcRequest = serde_json::from_str(r#"{"kind":"host_stop"}"#).unwrap();
        assert_eq!(stop, ScreenIpcRequest::HostStop { session: None });
        let reply = ScreenIpcReply::Stopped { count: 2 };
        assert_eq!(
            serde_json::to_string(&reply).unwrap(),
            r#"{"kind":"stopped","count":2}"#
        );

        // Relay frames keep their length limit before allocating.
        let mut wire = Vec::new();
        write_relay(&mut wire, RELAY_CLOCK, &(-5i64).to_be_bytes()).unwrap();
        let (tag, payload) = read_relay(&mut wire.as_slice(), 8).unwrap();
        assert_eq!((tag, payload.len()), (RELAY_CLOCK, 8));
        let too_long = read_relay(&mut wire.as_slice(), 7).unwrap_err();
        assert_eq!(too_long.kind(), std::io::ErrorKind::InvalidData);

        // A picture record crosses the relay unchanged; a length that does
        // not match is refused.
        let header = GopHeader {
            family_layout_revision: None,
            frame_contract: None,
            gop_id: 7,
            kind: crate::screen::core::wire_binary::StreamKind::Video,
            codec: crate::screen::core::negotiate::CodecKind::Tiles,
            profile: None,
            size: FrameSize::new(640, 360),
        };
        let record = FrameRecord {
            frame_id: 42,
            capture_ts_us: 1_000,
            flags: crate::screen::core::wire_binary::FLAG_KEYFRAME,
            payload: vec![1, 2, 3],
        };
        let body = encode_relay_record(&header, &record).unwrap();
        assert_eq!(
            decode_relay_record(&body).unwrap(),
            (header, record.clone())
        );
        let cut = &body[..body.len() - 1];
        assert!(decode_relay_record(cut).is_err());

        // A family layout belongs to the actual v2 picture header across IPC.
        // Legacy v2 extensions still have no layout revision.
        // Versioned limits stay within the 24-image budget (d577bab8).
        use crate::screen::core::frame_budget::{FrameCapabilities, FrameContract, FRAME_PROTOCOL};
        let limits = FrameCapabilities::legacy_resources(FrameSize::new(4096, 4096)).frames;
        let mut header = header;
        header.frame_contract = Some(FrameContract {
            version: FRAME_PROTOCOL,
            limits,
            hybrid_regions: true,
        });
        let body = encode_relay_record(&header, &record).unwrap();
        assert_eq!(
            decode_relay_record(&body).unwrap(),
            (header, record.clone())
        );
        header.family_layout_revision = Some(u64::MAX);
        let body = encode_relay_record(&header, &record).unwrap();
        assert_eq!(decode_relay_record(&body).unwrap(), (header, record));
        header.family_layout_revision = Some(0);
        assert!(encode_relay_record(
            &header,
            &FrameRecord {
                frame_id: 43,
                capture_ts_us: 1_001,
                flags: 1,
                payload: vec![1],
            }
        )
        .is_err());
    }
}
