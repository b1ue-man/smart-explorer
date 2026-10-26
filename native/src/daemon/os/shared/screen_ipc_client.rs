//! Client calls of `IpcRequest::Screen` (SCR-M2, strand S4) for `se screen`,
//! the tab „Bildschirm“, the indicator window and the desktop viewer. Probe,
//! measurement and the relayed session start the background worker when
//! needed; reading or ending this host's sessions never starts one (without
//! a worker there are none). A measurement waits without a time limit while
//! the other device's user decides (C5) and reports each `Pending`.
use std::io;
use std::net::TcpStream;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use super::ipc_protocol::screen::{
    decode_relay_record, read_relay_budgeted, write_relay, BenchRequest, RelayEnd, RelayHello,
    ScreenIpcReply, ScreenIpcRequest, StreamRequest, MAX_RELAY_BYTES, RELAY_BROKEN, RELAY_CLOCK,
    RELAY_CLOSE, RELAY_END, RELAY_HELLO, RELAY_HOST, RELAY_RECORD, RELAY_VIEWER, WATCH_WAIT_SECS,
};
use super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse,
};
use crate::screen::core::bench::{BenchOptions, BenchReport};
use crate::screen::core::frame_budget::{FrameContract,FrameLimits};
use crate::screen::core::frame_size_caps::PacketBudget;
use crate::screen::core::registry::SessionView;
use crate::screen::core::wire::from_json;
use crate::screen::core::wire_binary::{FrameRecord, GopHeader, GOP_HEADER_BYTES, RECORD_HEADER_BYTES};
use crate::screen::core::wire_control::{
    decode_host_frame, encode_viewer_frame, HostFrame, ViewerFrame,
};
use crate::screen::core::wire_hello::ScreenHelloOk;
use crate::share::access_grant::Capability;
use crate::share::PeerOpenTarget;

/// What `se screen probe` and `se screen devices --probe` show of a device.
pub(crate) use super::ipc_protocol::screen::ProbeReport as ScreenProbeReport;

/// Answer time of the host's own requests (list, stop).
const HOST_TIMEOUT: Duration = Duration::from_secs(20);
/// A probe connects to another device (presence, relay) first.
const PROBE_TIMEOUT: Duration = Duration::from_secs(90);

fn unexpected() -> String {
    "Unerwartete Worker-Antwort auf eine Bildschirmanfrage".into()
}

/// The text of a refusal; a screen refusal (C7) leads with its code.
fn refusal(msg: String, code: Option<String>) -> String {
    match code {
        Some(code) => format!("{code}: {msg}"),
        None => msg,
    }
}

fn connect() -> Result<TcpStream, String> {
    let addr = super::ipc_storage::read_ipc_addr()
        .ok_or_else(|| "Background-Worker IPC nicht bereit".to_string())?;
    TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|error| format!("Background-Worker IPC: {error}"))
}

/// Sends `req` on a fresh connection.
fn open(req: ScreenIpcRequest, timeout: Option<Duration>) -> Result<TcpStream, String> {
    let token = super::ipc_storage::read_token()
        .map_err(|error| format!("Background-Worker Token: {error}"))?;
    let mut stream = connect()?;
    set_stream_timeout(&stream, timeout);
    write_request(&mut stream, &IpcRequest::Screen { token, req })
        .map_err(|error| error.to_string())?;
    Ok(stream)
}

/// One request and its final reply; each `Pending` on the way goes to
/// `pending`.
fn exchange(
    req: ScreenIpcRequest,
    timeout: Option<Duration>,
    mut pending: impl FnMut(Capability),
) -> Result<ScreenIpcReply, String> {
    let mut stream = open(req, timeout)?;
    loop {
        match read_response(&mut stream).map_err(|error| error.to_string())? {
            IpcResponse::Screen(ScreenIpcReply::Pending { capability }) => pending(capability),
            IpcResponse::Screen(reply) => return Ok(reply),
            IpcResponse::Err { msg, code } => return Err(refusal(msg, code)),
            _ => return Err(unexpected()),
        }
    }
}

fn worker_running() -> bool {
    super::ipc_storage::read_ipc_addr().is_some()
}

/// Rights, platform, codecs and limits of `target`'s screen.
pub(crate) fn screen_probe(target: PeerOpenTarget) -> Result<ScreenProbeReport, String> {
    crate::daemon::ensure_worker_ready()?;
    match exchange(
        ScreenIpcRequest::Probe { target },
        Some(PROBE_TIMEOUT),
        |_| {},
    )? {
        ScreenIpcReply::Probe(report) => Ok(report),
        _ => Err(unexpected()),
    }
}

/// `se screen bench` with `target`; `pending` hears each confirmation the
/// other device's user is asked for.
pub(crate) fn screen_bench(
    target: PeerOpenTarget,
    options: &BenchOptions,
    pending: impl FnMut(Capability),
) -> Result<BenchReport, String> {
    crate::daemon::ensure_worker_ready()?;
    let options = BenchRequest {
        codec: options.codec.key().into(),
        size: options.size,
        fps: options.fps,
        seconds: options.seconds,
    };
    match exchange(ScreenIpcRequest::Bench { target, options }, None, pending)? {
        ScreenIpcReply::Bench(report) => Ok(report),
        _ => Err(unexpected()),
    }
}

/// The revision and the sessions of this host; none without a worker.
pub(crate) fn screen_sessions() -> Result<(u64, Vec<SessionView>), String> {
    if !worker_running() {
        return Ok((0, Vec::new()));
    }
    sessions_reply(exchange(
        ScreenIpcRequest::HostSessions,
        Some(HOST_TIMEOUT),
        |_| {},
    )?)
}

/// The sessions once their revision passed `since` (at most 25 s).
pub(crate) fn screen_watch(since: u64) -> Result<(u64, Vec<SessionView>), String> {
    if !worker_running() {
        return Err("Background-Worker IPC nicht bereit".into());
    }
    let wait = Duration::from_secs(WATCH_WAIT_SECS) + HOST_TIMEOUT;
    sessions_reply(exchange(
        ScreenIpcRequest::HostWatch { since },
        Some(wait),
        |_| {},
    )?)
}

fn sessions_reply(reply: ScreenIpcReply) -> Result<(u64, Vec<SessionView>), String> {
    match reply {
        ScreenIpcReply::Sessions { revision, sessions } => Ok((revision, sessions)),
        _ => Err(unexpected()),
    }
}

/// Ends the session `session` of this host, or all without it; the count.
pub(crate) fn screen_stop(session: Option<String>) -> Result<usize, String> {
    if !worker_running() {
        return Ok(0);
    }
    match exchange(
        ScreenIpcRequest::HostStop { session },
        Some(HOST_TIMEOUT),
        |_| {},
    )? {
        ScreenIpcReply::Stopped { count } => Ok(count),
        _ => Err(unexpected()),
    }
}

/// What a relayed viewer session delivers.
#[allow(dead_code)] // Staged: the desktop viewer of SCR-M7 (U25) reads the fields.
#[derive(Debug)]
pub(crate) enum StreamEvent {
    Pending(Capability),
    Accepted {
        hello: ScreenHelloOk,
        path: String,
    },
    Host(HostFrame),
    Record {
        header: GopHeader,
        record: FrameRecord,
    },
    Broken {
        gop_id: u64,
    },
    /// Host clock minus viewer clock in µs (`PING`/`PONG`).
    Clock(i64),
    End(RelayEnd),
}

/// A viewer session the worker relays (the desktop viewer of SCR-M7).
#[allow(dead_code)] // Staged: the desktop viewer of SCR-M7 (U25) holds it.
pub(crate) struct ScreenStream {
    events: TcpStream,
    input: Arc<Mutex<TcpStream>>,
    /// None until Accepted; Some(None) is an explicitly legacy session.
    frame_contract: Option<Option<FrameContract>>,
    local_limits:Option<FrameLimits>,
}

/// Opens a relayed viewer session with `target`.
#[allow(dead_code)] // Staged: the desktop viewer of SCR-M7 (U25) reads it.
pub(crate) fn screen_stream(
    target: PeerOpenTarget,
    request: StreamRequest,
) -> Result<ScreenStream, String> {
    if let Some(expected) = request.expected_node.as_deref() {
        require_node_pin(expected)?;
        open_stream(ScreenIpcRequest::PinnedStream {target,request})
    } else {
        open_stream(ScreenIpcRequest::Stream {target,request})
    }
}

pub(crate) fn screen_metadata_stream(target: PeerOpenTarget) -> Result<ScreenStream,String> {
    open_stream(ScreenIpcRequest::Metadata {target})
}

pub(crate) fn screen_metadata_stream_for_node(target: PeerOpenTarget, expected_node: &str) -> Result<ScreenStream,String> {
    require_node_pin(expected_node)?;
    open_stream(ScreenIpcRequest::PinnedMetadata {target, expected_node: expected_node.to_owned()})
}

fn require_node_pin(expected_node: &str) -> Result<(),String> {
    if expected_node.trim().is_empty() {
        return Err("screen_node_required: Der bestaetigte Nodepin fehlt".into());
    }
    Ok(())
}

fn open_stream(request: ScreenIpcRequest) -> Result<ScreenStream,String> {
    crate::daemon::ensure_worker_ready()?;
    let mut stream = open(
        request,
        Some(PROBE_TIMEOUT),
    )?;
    match read_response(&mut stream).map_err(|error| error.to_string())? {
        IpcResponse::Screen(ScreenIpcReply::StreamOpened) => {}
        IpcResponse::Err { msg, code } => return Err(refusal(msg, code)),
        _ => return Err(unexpected()),
    }
    set_stream_timeout(&stream, None);
    let input = stream.try_clone().map_err(|error| error.to_string())?;
    Ok(ScreenStream {
        events: stream,
        input: Arc::new(Mutex::new(input)),
        frame_contract: None,
        local_limits:None,
    })
}

#[allow(dead_code)] // Staged: the desktop viewer of SCR-M7 (U25) drives it.
impl ScreenStream {
    /// The next event; blocks until one arrives.
    pub(crate) fn next_event(&mut self) -> io::Result<StreamEvent> {
        let record_limit = match self.frame_contract {
            Some(contract) => PacketBudget::for_receive(contract,self.local_limits)?.record_bytes
                + GOP_HEADER_BYTES + RECORD_HEADER_BYTES + if contract.is_some() {1026} else {0},
            None => 0,
        };
        let (tag, payload) = read_relay_budgeted(&mut self.events, MAX_RELAY_BYTES, record_limit)?;
        let event = decode_event(tag, &payload)?;
        match &event {
            StreamEvent::Accepted { hello, .. } => {
                if self.frame_contract.is_some() {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "duplicate screen relay acceptance"));
                }
                if let Some(contract) = hello.frame_contract { contract.validate()?; }
                let local=match from_json::<RelayHello>(&payload)? {
                    RelayHello::Accepted {local_limits,..}=>local_limits,
                    RelayHello::Pending {..}=>None,
                };
                if let Some(limits)=local {
                    limits.validate()?;
                    if hello.frame_contract.is_some_and(|contract|contract.limits!=limits)
                        || (hello.frame_contract.is_none() && limits.intersect(FrameLimits::LEGACY)!=limits) {
                        return Err(io::Error::new(io::ErrorKind::InvalidData,"screen relay local limits differ from admission"));
                    }
                    PacketBudget::for_receive(hello.frame_contract,Some(limits))?;
                }
                self.local_limits=local.or(hello.frame_contract.map(|contract|contract.limits));
                self.frame_contract = Some(hello.frame_contract);
            }
            StreamEvent::Record { header, .. } => {
                let matches = match (header.frame_contract, self.frame_contract) {
                    (None, Some(None)) => true,
                    (Some(actual), Some(Some(accepted))) => actual.version == accepted.version
                        && actual.limits == accepted.limits
                        && (!actual.hybrid_regions || accepted.hybrid_regions),
                    _ => false,
                };
                if !matches {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "screen relay frame contract changed"));
                }
                if self.local_limits.is_some_and(|limits|limits.check(header.size).is_err()) {
                    return Err(io::Error::new(io::ErrorKind::InvalidData,"screen relay geometry exceeds local admission"));
                }
            }
            _ => {}
        }
        Ok(event)
    }
    pub(crate) fn local_frame_limits(&self)->Option<FrameLimits> {self.local_limits}

    /// Sends a control frame to the host (`SET_MODE`, `INPUT`, …).
    pub(crate) fn send(&self, frame: &ViewerFrame) -> io::Result<()> {
        self.control().send(frame)
    }

    /// Ends the session.
    pub(crate) fn close(&self) -> io::Result<()> {
        self.control().close()
    }

    /// The sending half, for another thread while `next_event` waits here
    /// (a call that gives up waiting for the other device's user, SCR-M4).
    pub(crate) fn control(&self) -> StreamControl {
        StreamControl {
            input: self.input.clone(),
        }
    }
}

/// Sends control frames of a relayed viewer session and ends it; it works
/// while another thread waits in `ScreenStream::next_event`.
#[derive(Clone)]
pub(crate) struct StreamControl {
    input: Arc<Mutex<TcpStream>>,
}

impl StreamControl {
    pub(crate) fn send(&self, frame: &ViewerFrame) -> io::Result<()> {
        let (tag, payload) = encode_viewer_frame(frame)?;
        let mut body = Vec::with_capacity(payload.len() + 1);
        body.push(tag);
        body.extend_from_slice(&payload);
        let mut input = self.input.lock().unwrap_or_else(PoisonError::into_inner);
        write_relay(&mut *input, RELAY_VIEWER, &body)
    }

    pub(crate) fn close(&self) -> io::Result<()> {
        let mut input = self.input.lock().unwrap_or_else(PoisonError::into_inner);
        write_relay(&mut *input, RELAY_CLOSE, &[])
    }
}

fn decode_event(tag: u8, payload: &[u8]) -> io::Result<StreamEvent> {
    let invalid = |text: &'static str| io::Error::new(io::ErrorKind::InvalidData, text);
    match tag {
        RELAY_HELLO => Ok(match from_json::<RelayHello>(payload)? {
            RelayHello::Pending { capability } => StreamEvent::Pending(capability),
            RelayHello::Accepted { hello, path, .. } => StreamEvent::Accepted { hello, path },
        }),
        RELAY_HOST => {
            let (wire_tag, body) = payload
                .split_first()
                .ok_or_else(|| invalid("empty screen relay frame"))?;
            decode_host_frame(*wire_tag, body).map(StreamEvent::Host)
        }
        RELAY_RECORD => {
            let (header, record) = decode_relay_record(payload)?;
            Ok(StreamEvent::Record { header, record })
        }
        RELAY_BROKEN | RELAY_CLOCK => {
            let bytes: [u8; 8] = payload
                .try_into()
                .map_err(|_| invalid("screen relay frame needs 8 bytes"))?;
            Ok(if tag == RELAY_BROKEN {
                StreamEvent::Broken {
                    gop_id: u64::from_be_bytes(bytes),
                }
            } else {
                StreamEvent::Clock(i64::from_be_bytes(bytes))
            })
        }
        RELAY_END => from_json(payload).map(StreamEvent::End),
        _ => Err(invalid("unknown screen relay frame")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_task_ipc_stream_events_decode() {
        let pending = crate::screen::core::wire::to_json(&RelayHello::Pending {
            capability: Capability::ScreenView,
        })
        .unwrap();
        assert!(matches!(
            decode_event(RELAY_HELLO, &pending).unwrap(),
            StreamEvent::Pending(Capability::ScreenView)
        ));
        assert!(matches!(
            decode_event(RELAY_CLOCK, &(-250i64).to_be_bytes()).unwrap(),
            StreamEvent::Clock(-250)
        ));
        assert!(matches!(
            decode_event(RELAY_BROKEN, &9u64.to_be_bytes()).unwrap(),
            StreamEvent::Broken { gop_id: 9 }
        ));
        let end = br#"{"code":"approval_denied"}"#;
        match decode_event(RELAY_END, end).unwrap() {
            StreamEvent::End(end) => assert_eq!(end.code, "approval_denied"),
            other => panic!("unexpected {other:?}"),
        }
        assert!(decode_event(RELAY_CLOCK, &[1, 2]).is_err());
        assert!(decode_event(99, &[]).is_err());
    }
}
