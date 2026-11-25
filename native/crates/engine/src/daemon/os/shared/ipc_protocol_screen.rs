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
pub(crate) mod switch;
pub use relay::{
    decode_relay_record, encode_relay_record, read_relay, read_relay_budgeted, write_relay,
};
pub use switch::{RateCeilingState, RATE_CEILING_UNSUPPORTED, RELAY_AUDIO, RELAY_RATE_CEILING};

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
    /// „Übertragung“ of the viewer (user 2026-11-25, `screen::core::delivery`):
    /// the hello's `delivery`; absent is the default (`realtime`). A daemon
    /// before it ignores the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<crate::screen::core::delivery::Delivery>,
}

impl StreamRequest {
    /// The decoders the hello names under „Codec“ and „Dekodierer“: tiles
    /// under „Automatisch“ (a chosen codec offers none, the host reads the
    /// choice from that), tiles alone for „Verlustfrei“ (fix 2026-11-19), no
    /// tiles (software) for a chosen „Hardware“ (Codex 4/15). „Verlustfrei“
    /// always offers the tiles (2026-11-24, user: „was fällt dir ein die
    /// Nutzereinstellung zu verweigern“): „Dekodierer“ chooses among video
    /// decoders, and the tiles have their one decoder on every device.
    pub fn chosen_decoders(&self) -> Vec<CodecOffer> {
        let setting = self.codec.unwrap_or_default();
        let decoder = self.decoder.unwrap_or_default();
        let codec = match setting {
            CodecSetting::Lossless => {
                return crate::screen::core::negotiate::CodecPreference::Tiles
                    .filter_decoders(&self.decoders);
            }
            setting => setting.preference(),
        };
        let mut offers = decoder.filter_offers(&codec.filter_decoders(&self.decoders));
        let hardware = decoder == DecoderPreference::Hardware;
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
#[path = "ipc_protocol_screen_tests.rs"]
mod tests;
