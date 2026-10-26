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
use crate::screen::core::negotiate::CodecOffer;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::registry::SessionView;
use crate::screen::core::window_list::{MonitorInfo, SourceId};
use crate::screen::core::wire::BeReader;
use crate::screen::core::wire_binary::{
    FrameRecord, GopHeader, GOP_HEADER_BYTES, RECORD_HEADER_BYTES,
};
use crate::screen::core::wire_hello::{HostPlatform, SessionMode};
use crate::share::access_grant::{Capability, CapabilityDecision};
use crate::share::PeerOpenTarget;

/// Longest a `HostWatch` waits for a change before it answers anyway.
pub(crate) const WATCH_WAIT_SECS: u64 = 25;

/// What a viewer session asks the host for (the client hello's fields).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StreamRequest {
    /// Authenticated node of an app-start receipt; never inferred from a fresh target lookup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) expected_node: Option<String>,
    pub(crate) mode: SessionMode,
    #[serde(default)]
    pub(crate) source: Option<SourceId>,
    #[serde(default)]
    pub(crate) decoders: Vec<CodecOffer>,
    #[serde(default)]
    pub(crate) display_size: Option<FrameSize>,
    #[serde(default)]
    pub(crate) frame_capabilities: Option<crate::screen::core::frame_budget::FrameCapabilities>,
    pub(crate) max_fps: u16,
}

/// `se screen bench` options as the worker takes them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BenchRequest {
    /// `auto`, `h264` or `tiles`.
    pub(crate) codec: String,
    pub(crate) size: FrameSize,
    pub(crate) fps: u16,
    pub(crate) seconds: u32,
}

/// One request of the screen strand.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ScreenIpcRequest {
    /// Rights, platform, codecs, monitors and limits of `target`'s screen.
    Probe { target: PeerOpenTarget },
    /// List/icons without opening the host's capture source.
    Metadata { target: PeerOpenTarget },
    /// An old daemon must reject the variant instead of silently ignoring the pin.
    PinnedMetadata { target: PeerOpenTarget, expected_node: String },
    /// A viewer session relayed over this connection.
    Stream {
        target: PeerOpenTarget,
        request: StreamRequest,
    },
    /// Requires a nonempty expected_node in request before opening the stream.
    PinnedStream { target: PeerOpenTarget, request: StreamRequest },
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
pub(crate) struct ProbeReport {
    /// How this device names the other one.
    pub(crate) device: String,
    /// Whether it knows the screen protocol of this version.
    pub(crate) supported: bool,
    pub(crate) platform: Option<HostPlatform>,
    pub(crate) view: CapabilityDecision,
    pub(crate) control: CapabilityDecision,
    pub(crate) monitors: Vec<MonitorInfo>,
    pub(crate) input: InputCapabilities,
    pub(crate) codecs: Vec<CodecOffer>,
    pub(crate) limits: Vec<String>,
}

/// The replies of `ScreenIpcRequest`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum ScreenIpcReply {
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
pub(crate) const RELAY_HELLO: u8 = 1;
pub(crate) const RELAY_HOST: u8 = 2;
pub(crate) const RELAY_RECORD: u8 = 3;
pub(crate) const RELAY_BROKEN: u8 = 4;
pub(crate) const RELAY_CLOCK: u8 = 5;
pub(crate) const RELAY_END: u8 = 6;
// Client to worker.
pub(crate) const RELAY_VIEWER: u8 = 16;
pub(crate) const RELAY_CLOSE: u8 = 17;

/// Largest relay payload: a picture record (16 MiB) with both heads.
pub(crate) const MAX_RELAY_BYTES: usize = 16 * 1024 * 1024 + 64 + 1026;
/// Largest relay payload from the client: one viewer control frame.
pub(crate) const MAX_RELAY_VIEWER_BYTES: usize = 64 * 1024 + 1;

/// The end of a relayed session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RelayEnd {
    pub(crate) code: String,
    #[serde(default)]
    pub(crate) detail: Option<String>,
}

/// The reply of the host's handshake as the relay forwards it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RelayHello {
    Pending {
        capability: Capability,
    },
    Accepted {
        hello: crate::screen::core::wire_hello::ScreenHelloOk,
        /// `direct` or `relay`.
        path: String,
        /// Private daemon allocation evidence; absent from the peer HELLO_OK.
        #[serde(default,skip_serializing_if="Option::is_none")]
        local_limits:Option<crate::screen::core::frame_budget::FrameLimits>,
    },
}

/// Same flattened v2 extension as the remote picture stream. Legacy v2 JSON
/// without a family revision retains exactly its existing meaning.
#[derive(Serialize, Deserialize)]
struct RelayGopExtension {
    #[serde(flatten)]
    contract: crate::screen::core::frame_budget::FrameContract,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    family_layout_revision: Option<u64>,
}

/// The relay payload of one picture record: its picture group's head, the
/// record head and the payload.
pub(crate) fn encode_relay_record(
    header: &GopHeader,
    record: &FrameRecord,
) -> std::io::Result<Vec<u8>> {
    let len = u32::try_from(record.payload.len())
        .map_err(|_| std::io::Error::other("picture record is too large"))?;
    let head = header.encode()?;
    if record.payload.len() > header.record_limit() {
        return Err(std::io::Error::other("picture record exceeds its frame contract"));
    }
    let extension = if let Some(contract) = header.frame_contract {
        let extension = RelayGopExtension { contract, family_layout_revision: header.family_layout_revision };
        let bytes = serde_json::to_vec(&extension).map_err(std::io::Error::other)?;
        if bytes.len() > 1024 {
            return Err(std::io::Error::other("screen relay contract exceeds 1 KiB"));
        }
        Some(bytes)
    } else { None };
    let overhead = GOP_HEADER_BYTES + RECORD_HEADER_BYTES
        + extension.as_ref().map_or(0, |bytes| bytes.len() + 2);
    let capacity = record.payload.len().checked_add(overhead)
        .ok_or_else(|| std::io::Error::other("screen relay record length overflow"))?;
    let mut body = Vec::with_capacity(capacity);
    body.extend_from_slice(&head);
    if let Some(bytes) = extension {
        body.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
        body.extend_from_slice(&bytes);
    }
    body.extend_from_slice(&len.to_be_bytes());
    body.extend_from_slice(&record.frame_id.to_be_bytes());
    body.extend_from_slice(&record.capture_ts_us.to_be_bytes());
    body.push(record.flags);
    body.extend_from_slice(&record.payload);
    Ok(body)
}

/// The inverse of `encode_relay_record`; the record length must match and
/// stay within the limit of its codec.
pub(crate) fn decode_relay_record(payload: &[u8]) -> std::io::Result<(GopHeader, FrameRecord)> {
    let invalid = |text: &'static str| std::io::Error::new(std::io::ErrorKind::InvalidData, text);
    let (head, rest) = payload
        .split_at_checked(GOP_HEADER_BYTES)
        .ok_or_else(|| invalid("screen relay record is too short"))?;
    let mut bytes = [0u8; GOP_HEADER_BYTES];
    bytes.copy_from_slice(head);
    let (mut header,extended) = GopHeader::decode_prefix(&bytes)?;
    let rest = if extended {
        let (length, data) = rest.split_at_checked(2)
            .ok_or_else(|| invalid("screen relay contract length is missing"))?;
        let length = u16::from_be_bytes([length[0], length[1]]) as usize;
        if length > 1024 {
            return Err(invalid("screen relay contract exceeds 1 KiB"));
        }
        let (json, remaining) = data.split_at_checked(length)
            .ok_or_else(|| invalid("screen relay contract is truncated"))?;
        let extension: RelayGopExtension =
            serde_json::from_slice(json).map_err(std::io::Error::other)?;
        header.frame_contract = Some(extension.contract);
        header.family_layout_revision = extension.family_layout_revision;
        // Reuse the header's full validation, including a nonzero family
        // revision bound to a v2 contract, without altering the received values.
        header.encode()?;
        remaining
    } else {
        rest
    };
    let (fields, data) = rest
        .split_at_checked(RECORD_HEADER_BYTES)
        .ok_or_else(|| invalid("screen relay record is too short"))?;
    let mut reader = BeReader::new(fields);
    let len = reader.u32()? as usize;
    let frame_id = reader.u64()?;
    let capture_ts_us = reader.u64()?;
    let flags = reader.u8()?;
    if len != data.len() || len > header.record_limit() {
        return Err(invalid("screen relay record length does not match"));
    }
    let record = FrameRecord {
        frame_id,
        capture_ts_us,
        flags,
        payload: data.to_vec(),
    };
    Ok((header, record))
}

/// Writes one relay frame.
pub(crate) fn write_relay(
    writer: &mut impl std::io::Write,
    tag: u8,
    payload: &[u8],
) -> std::io::Result<()> {
    let len = u32::try_from(payload.len())
        .map_err(|_| std::io::Error::other("screen relay frame is too large"))?;
    writer.write_all(&[tag])?;
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(payload)?;
    writer.flush()
}

/// Reads one relay frame of at most `limit` payload bytes; the length is
/// checked before memory for it is requested.
pub(crate) fn read_relay(
    reader: &mut impl std::io::Read,
    limit: usize,
) -> std::io::Result<(u8, Vec<u8>)> {
    read_relay_budgeted(reader, limit, limit)
}

/// Picture records have the accepted session's tighter allocation ceiling;
/// control frames retain their existing independent protocol limit.
pub(crate) fn read_relay_budgeted(
    reader: &mut impl std::io::Read,
    limit: usize,
    record_limit: usize,
) -> std::io::Result<(u8, Vec<u8>)> {
    let mut head = [0u8; 5];
    reader.read_exact(&mut head)?;
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len > limit || (head[0] == RELAY_RECORD && len > record_limit) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "screen relay frame exceeds its limit",
        ));
    }
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload)?;
    Ok((head[0], payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batches_task_b6_session_budget_private_relay_keeps_v1_and_rejects_oversize_before_read() {
        use crate::screen::core::{frame_budget::legacy_limits,frame_size_caps::PacketBudget};
        let local=legacy_limits(24*1024*1024,None).unwrap();
        let hello=serde_json::from_value(serde_json::json!({"authorization":{"view":"allow","control":"allow"},
            "platform":"windows","approval":"not_required"})).unwrap();
        let accepted=RelayHello::Accepted {hello,path:"direct".into(),local_limits:Some(local)};
        let bytes=serde_json::to_vec(&accepted).unwrap();let accepted:RelayHello=serde_json::from_slice(&bytes).unwrap();
        let RelayHello::Accepted {hello,local_limits,..}=accepted else {panic!("not Accepted")};
        assert!(hello.frame_contract.is_none());assert_eq!(local_limits,Some(local));
        let packet=PacketBudget::for_receive(hello.frame_contract,local_limits).unwrap();
        let bound=packet.record_bytes+GOP_HEADER_BYTES+RECORD_HEADER_BYTES;
        let mut bytes=vec![RELAY_RECORD];bytes.extend_from_slice(&((bound+1) as u32).to_be_bytes());
        let error=read_relay_budgeted(&mut bytes.as_slice(),usize::MAX,bound).unwrap_err();
        assert_eq!(error.kind(),std::io::ErrorKind::InvalidData,"no payload was supplied");
    }

    #[test]
    fn batches_task_b6_extended_4k_ipc_contract_is_complete() {
        use crate::screen::core::frame_budget::{FrameCapabilities,FrameContract};
        let caps=FrameCapabilities::admitted(FrameSize::new(16384,16384),768*1024*1024).unwrap();
        let contract=FrameContract::negotiate(Some(caps),Some(caps)).unwrap();
        let header=GopHeader {gop_id:1,frame_contract:contract,family_layout_revision:None,
            kind:crate::screen::core::wire_binary::StreamKind::Video,
            codec:crate::screen::core::negotiate::CodecKind::Tiles,profile:None,size:FrameSize::new(3840,2160)};
        let record=FrameRecord {frame_id:1,capture_ts_us:100,
            flags:crate::screen::core::wire_binary::FLAG_KEYFRAME,payload:vec![0]};
        let bytes=encode_relay_record(&header,&record).unwrap();
        assert_eq!(decode_relay_record(&bytes).unwrap(),(header,record));
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
        assert_eq!(decode_relay_record(&body).unwrap(), (header, record.clone()));
        let cut = &body[..body.len() - 1];
        assert!(decode_relay_record(cut).is_err());

        // A family layout belongs to the actual v2 picture header across IPC.
        // Legacy v2 extensions still have no layout revision.
        let mut header = header;
        header.frame_contract = Some(crate::screen::core::frame_budget::FrameContract {
            version: crate::screen::core::frame_budget::FRAME_PROTOCOL,
            limits: crate::screen::core::frame_budget::FrameLimits::LEGACY,
            hybrid_regions: true,
        });
        let body = encode_relay_record(&header, &record).unwrap();
        assert_eq!(decode_relay_record(&body).unwrap(), (header, record.clone()));
        header.family_layout_revision = Some(u64::MAX);
        let body = encode_relay_record(&header, &record).unwrap();
        assert_eq!(decode_relay_record(&body).unwrap(), (header, record));
        header.family_layout_revision = Some(0);
        assert!(encode_relay_record(&header, &FrameRecord {
            frame_id: 43, capture_ts_us: 1_001, flags: 1, payload: vec![1],
        }).is_err());
    }
}
