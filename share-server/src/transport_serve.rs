//! Post-registration loops of both signaling transports. They share one
//! liveness timing ([`SignalSession`]): blocking reads end at the next
//! keepalive or read deadline, a raw TCP line split across such wake-ups is
//! kept, and every inbound byte or frame counts as client activity. Clients
//! that are not idle close after 60 s of silence on both transports.
//! WebSocket I/O waits on readiness and outbound notification, without polling.

use std::io::{self, BufReader, ErrorKind};
use std::net::TcpStream;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::{
    flush_websocket_out, read_websocket_json_until, require_inbound_budget, SignalingWebSocket,
};
use crate::idle::{SignalTiming, ACTIVE_READ_WINDOW};
use crate::line::{LineRead, LineReader, MAX_JSON_LINE};
use crate::rate_limits::InboundRateLimiter;
use crate::signal_session::{SignalSession, Step};
use crate::state::State;
use crate::writer::QueuedMessage;
use crate::{dispatch, In, Writer};

pub(super) fn serve_tcp(
    reader: &mut BufReader<TcpStream>,
    id: u64,
    writer: &Writer,
    state: &Arc<Mutex<State>>,
    timing: &SignalTiming,
    inbound_rate: &mut InboundRateLimiter,
) -> io::Result<()> {
    let mut lines = LineReader::default();
    let mut session = SignalSession::new(timing.clock.now(), Some(ACTIVE_READ_WINDOW));
    loop {
        let now = timing.clock.now();
        let Step::Wait(until) = session.poll(writer, now) else {
            return Ok(());
        };
        reader
            .get_ref()
            .set_read_timeout(timing.read_timeout(now, until))?;
        let read = lines.read(reader, MAX_JSON_LINE);
        if lines.take_received() {
            session.inbound(timing.clock.now());
        }
        let line = match read {
            Ok(LineRead::Line(line)) => line,
            Ok(LineRead::Eof) => return Ok(()),
            Err(error) if is_wait_expiry(&error) => continue,
            Err(_) => return Ok(()),
        };
        require_inbound_budget(inbound_rate, line.len())?;
        if let Ok(message) = serde_json::from_str::<In>(line.trim()) {
            dispatch(id, writer, message, state);
        }
    }
}

pub(super) fn serve_websocket(
    websocket: &mut SignalingWebSocket,
    outbound: &Receiver<QueuedMessage>,
    writer: &Writer,
    id: u64,
    state: &Arc<Mutex<State>>,
    timing: &SignalTiming,
    inbound_rate: &mut InboundRateLimiter,
) -> io::Result<()> {
    let mut session = SignalSession::new(timing.clock.now(), Some(ACTIVE_READ_WINDOW));
    loop {
        let now = timing.clock.now();
        let Step::Wait(until) = session.poll(writer, now) else {
            return Ok(());
        };
        flush_websocket_out(websocket, outbound)?;
        let wait = timing
            .read_timeout(now, until)
            .unwrap_or(ACTIVE_READ_WINDOW);
        websocket.get_mut().set_read_timeout(Some(wait))?;
        let mut received = false;
        let read = read_websocket_json_until(
            websocket,
            outbound,
            Instant::now() + wait,
            inbound_rate,
            &mut received,
        );
        if received {
            session.inbound(timing.clock.now());
        }
        match read {
            Ok(Some(message)) => dispatch(id, writer, message, state),
            Ok(None) => return Ok(()),
            Err(error) if is_wait_expiry(&error) => {}
            Err(error) => return Err(error),
        }
    }
}

/// A bounded read ended without a complete message; poll deadlines again.
fn is_wait_expiry(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
    )
}
