//! Window lists and still pictures of another device through the relayed
//! viewer session (SCR-M4, U16-5): `se screen windows`, `se screen snapshot`
//! and the MCP tool `screen`. Lists use a negotiated metadata channel;
//! older hosts receive LIST_WINDOWS over their synthetic source. Neither
//! list path opens screen capture. Snapshots retain the view stream and
//! SNAPSHOT request. Each call ends its session. Until the host accepts, nothing
//! limits the wait (its user may be asked, C7) unless the caller sets
//! `accept_within`; then the session is ended and the call fails with
//! `approval_pending`. After the acceptance the answer takes at most
//! `READ_TIMEOUT`: the host answers `LIST_WINDOWS` from the window list and
//! `SNAPSHOT` from one capture, which the host process bounds to 10 s, plus
//! margin. The session is read on a thread of its own so these limits hold
//! while the relay is silent.
use std::io;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use super::ipc_protocol::screen::{RelayEnd, StreamRequest};
use super::screen_ipc_client::{
    screen_metadata_stream, screen_metadata_stream_for_node, screen_stream, StreamControl,
    StreamEvent,
};
use crate::screen::core::app_icon::{IconReply, IconRequest};
use crate::screen::core::negotiate::CodecOffer;
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::snapshot::decode_snapshot_with_limits;
use crate::screen::core::viewer_client::refusal_text;
use crate::screen::core::window_list::{SourceId, WindowList};
use crate::screen::core::wire_binary::StreamKind;
use crate::screen::core::wire_control::{HostFrame, ScreenWireError, SnapshotRequest, ViewerFrame};
use crate::screen::core::wire_hello::{ScreenHelloOk, SessionMode};
use crate::share::access_grant::Capability;
use crate::share::PeerOpenTarget;

/// Time the host has for its answer once it accepted the session.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// A still picture and the source it shows (`monitor:<id>`, `window:<id>`).
pub(crate) struct StillImage {
    pub image: BgraImage,
    pub source: String,
}

/// What these calls need of a relayed session.
trait StillSession {
    /// The next event; `None` when `wait` passed without one (`None` waits
    /// without limit).
    fn next(&mut self, wait: Option<Duration>) -> io::Result<Option<StreamEvent>>;
    fn send(&self, frame: &ViewerFrame) -> io::Result<()>;
    /// Ends the session (the other device sees it cancelled or closed).
    fn close(&self);
}

/// The relayed session of the background worker, read on its own thread.
struct Relayed {
    events: Receiver<io::Result<StreamEvent>>,
    control: StreamControl,
}

impl Relayed {
    fn open(
        target: PeerOpenTarget,
        source: Option<SourceId>,
        metadata: bool,
    ) -> Result<Self, String> {
        Self::open_for_node(target, source, metadata, None)
    }

    fn open_for_node(
        target: PeerOpenTarget,
        source: Option<SourceId>,
        metadata: bool,
        expected_node: Option<&str>,
    ) -> Result<Self, String> {
        let request = StreamRequest {
            expected_node: expected_node.map(str::to_owned),
            mode: SessionMode::View,
            source,
            decoders: vec![CodecOffer::tiles()],
            display_size: None,
            max_fps: 1,
            frame_capabilities: None,
        };
        let mut stream = if metadata {
            match expected_node {
                Some(expected) => screen_metadata_stream_for_node(target, expected)?,
                None => screen_metadata_stream(target)?,
            }
        } else {
            screen_stream(target, request)?
        };
        let control = stream.control();
        let (sender, events) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("screen-still-reader".into())
            .spawn(move || loop {
                let event = stream.next_event();
                let last = matches!(event, Ok(StreamEvent::End(_)) | Err(_));
                if sender.send(event).is_err() || last {
                    return;
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self { events, control })
    }
}

impl StillSession for Relayed {
    fn next(&mut self, wait: Option<Duration>) -> io::Result<Option<StreamEvent>> {
        let received = match wait {
            Some(wait) => self.events.recv_timeout(wait),
            None => self
                .events
                .recv()
                .map_err(|_| RecvTimeoutError::Disconnected),
        };
        match received {
            Ok(event) => event.map(Some),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "Die Bildschirmsitzung endete unerwartet",
            )),
        }
    }

    fn send(&self, frame: &ViewerFrame) -> io::Result<()> {
        self.control.send(frame)
    }

    fn close(&self) {
        let _ = self.control.close();
    }
}

impl Drop for Relayed {
    fn drop(&mut self) {
        self.close();
    }
}

/// `code: text`, the form every refusal of `se screen` has.
fn coded(code: &str, detail: Option<&str>) -> String {
    let text = detail.map_or_else(|| refusal_text(code), str::to_string);
    // A code without a text of its own (a newer host's, `viewer_closed`) stands alone.
    if text == code {
        return text;
    }
    format!("{code}: {text}")
}

fn end_text(end: &RelayEnd) -> String {
    coded(&end.code, end.detail.as_deref())
}

fn error_text(error: &ScreenWireError) -> String {
    coded(&error.code, error.detail.as_deref())
}

/// The call gave up waiting for the other device's user.
fn pending_text(wait: Duration) -> String {
    format!(
        "approval_pending: Das Gerät fragt seinen Nutzer noch … Die Rückfrage wurde nach {} s zurückgezogen.",
        wait.as_secs()
    )
}

fn slow_text() -> String {
    format!(
        "capture_failed: Das Gerät antwortete nicht innerhalb von {} s",
        READ_TIMEOUT.as_secs()
    )
}

/// Waits for the host's acceptance; each `Pending` goes to `pending`.
fn accept(
    session: &mut dyn StillSession,
    accept_within: Option<Duration>,
    pending: &mut dyn FnMut(Capability),
) -> Result<ScreenHelloOk, String> {
    let started = Instant::now();
    loop {
        let wait = accept_within.map(|limit| limit.saturating_sub(started.elapsed()));
        match session.next(wait).map_err(|error| error.to_string())? {
            None => {
                session.close();
                return Err(pending_text(accept_within.unwrap_or_default()));
            }
            Some(StreamEvent::Pending(capability)) => pending(capability),
            Some(StreamEvent::Accepted { hello, .. }) => return Ok(hello),
            Some(StreamEvent::End(end)) => return Err(end_text(&end)),
            Some(StreamEvent::Host(HostFrame::Error(error))) => return Err(error_text(&error)),
            Some(_) => {}
        }
    }
}

/// The next event before `deadline`; later the session is ended.
fn event_before(session: &mut dyn StillSession, deadline: Instant) -> Result<StreamEvent, String> {
    let wait = deadline.saturating_duration_since(Instant::now());
    match session
        .next(Some(wait))
        .map_err(|error| error.to_string())?
    {
        Some(event) => Ok(event),
        None => {
            session.close();
            Err(slow_text())
        }
    }
}

fn windows_over(
    session: &mut dyn StillSession,
    accept_within: Option<Duration>,
    read: Duration,
    pending: &mut dyn FnMut(Capability),
) -> Result<WindowList, String> {
    accept(session, accept_within, pending)?;
    session
        .send(&ViewerFrame::ListWindows)
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + read;
    loop {
        match event_before(session, deadline)? {
            StreamEvent::Host(HostFrame::Windows(list)) => {
                session.close();
                return Ok(list);
            }
            StreamEvent::Host(HostFrame::Error(error)) => return Err(error_text(&error)),
            StreamEvent::End(end) => return Err(end_text(&end)),
            _ => {}
        }
    }
}

/// The name of the source of a session that named none: the primary monitor.
fn primary_source(hello: &ScreenHelloOk) -> String {
    hello
        .monitors
        .iter()
        .find(|monitor| monitor.primary)
        .or(hello.monitors.first())
        .map_or_else(
            || "primary".into(),
            |monitor| format!("monitor:{}", monitor.id),
        )
}

fn snapshot_over(
    session: &mut dyn StillSession,
    source: Option<SourceId>,
    max_size: Option<FrameSize>,
    accept_within: Option<Duration>,
    read: Duration,
    pending: &mut dyn FnMut(Capability),
) -> Result<StillImage, String> {
    let hello = accept(session, accept_within, pending)?;
    // The host shows the source of the session; the frame's own `source`
    // stays empty (U16-5).
    let request = SnapshotRequest {
        source: None,
        max_size,
    };
    session
        .send(&ViewerFrame::Snapshot(request))
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + read;
    let mut records = Vec::new();
    let mut stream = None;
    let header = loop {
        match event_before(session, deadline)? {
            StreamEvent::Record { header, record } if header.kind == StreamKind::Snapshot => {
                let accepted_header = *stream.get_or_insert(header);
                if accepted_header.gop_id != header.gop_id {
                    continue;
                }
                if accepted_header != header {
                    return Err("snapshot picture metadata changed within one group".into());
                }
                if header.frame_contract != hello.frame_contract {
                    return Err("snapshot frame contract differs from the accepted session".into());
                }
                let more = record.has_more();
                records.push(record);
                if !more {
                    break header;
                }
            }
            StreamEvent::Host(HostFrame::Error(error)) => return Err(error_text(&error)),
            StreamEvent::End(end) => return Err(end_text(&end)),
            _ => {}
        }
    };
    session.close();
    let limits = header.frame_contract.map_or(
        crate::screen::core::frame_budget::FrameLimits::LEGACY,
        |contract| contract.limits,
    );
    let image = decode_snapshot_with_limits(&records, header.size, limits)
        .map_err(|error| error.to_string())?;
    let source = source.map_or_else(|| primary_source(&hello), |source| source.to_string());
    Ok(StillImage { image, source })
}

/// The windows of `target`'s screen host.
pub(crate) fn screen_windows(
    target: PeerOpenTarget,
    pending: impl FnMut(Capability),
) -> Result<WindowList, String> {
    screen_windows_within(target, None, pending)
}

/// `screen_windows` that gives up after `accept_within` without the host's
/// acceptance (`approval_pending`).
pub(crate) fn screen_windows_within(
    target: PeerOpenTarget,
    accept_within: Option<Duration>,
    mut pending: impl FnMut(Capability),
) -> Result<WindowList, String> {
    let mut session = Relayed::open(target, None, true)?;
    windows_over(&mut session, accept_within, READ_TIMEOUT, &mut pending)
}

/// One still picture of `source` (the primary monitor without one) on
/// `target`, at most `max_size` large.
pub(crate) fn screen_snapshot(
    target: PeerOpenTarget,
    source: Option<SourceId>,
    max_size: Option<FrameSize>,
    pending: impl FnMut(Capability),
) -> Result<StillImage, String> {
    screen_snapshot_within(target, source, max_size, None, pending)
}

/// `screen_snapshot` that gives up after `accept_within` without the host's
/// acceptance (`approval_pending`).
pub(crate) fn screen_snapshot_within(
    target: PeerOpenTarget,
    source: Option<SourceId>,
    max_size: Option<FrameSize>,
    accept_within: Option<Duration>,
    mut pending: impl FnMut(Capability),
) -> Result<StillImage, String> {
    let mut session = Relayed::open(target, source.clone(), false)?;
    snapshot_over(
        &mut session,
        source,
        max_size,
        accept_within,
        READ_TIMEOUT,
        &mut pending,
    )
}

#[cfg(test)]
#[path = "screen_ipc_still_tests.rs"]
mod tests;

#[path = "screen_ipc_families.rs"]
mod families;
pub(crate) use families::{screen_families, screen_families_for_node};

/// A current authorized list target's PNG; unavailable or older peers return None.
pub(crate) fn screen_icon(
    target: PeerOpenTarget,
    request: IconRequest,
) -> Result<Option<IconReply>, String> {
    request.validate().map_err(|error| error.to_string())?;
    let mut session = Relayed::open(target, None, true)?;
    let hello = accept(&mut session, Some(READ_TIMEOUT), &mut |_| {})?;
    if hello.icon_version != Some(crate::screen::core::app_icon::ICON_VERSION) {
        return Ok(None);
    }
    session
        .send(&ViewerFrame::IconRequest(request.clone()))
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + READ_TIMEOUT;
    loop {
        match event_before(&mut session, deadline)? {
            StreamEvent::Host(HostFrame::IconReply(response)) if response.request == request => {
                response.validate().map_err(|error| error.to_string())?;
                return Ok(response.content);
            }
            StreamEvent::Host(HostFrame::Error(error)) => return Err(error_text(&error)),
            StreamEvent::End(end) => return Err(end_text(&end)),
            _ => {}
        }
    }
}
