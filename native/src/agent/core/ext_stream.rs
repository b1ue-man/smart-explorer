//! The reply stream of one extension request (`ext-v1`) and the small
//! helpers the extension calls share: send the request, hand every reply
//! frame to the caller until `End`, stop on `Err`, on a cancel or on a
//! closed transport.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::RecvTimeoutError;

use super::agent_error::agent_error;
use super::ext_wire::{is_unsupported, unsupported};
use super::pool::AgentPool;
use crate::agent_proto::Frame;

/// How often a waiting stream looks at its cancel flag.
pub(super) const CANCEL_POLL: Duration = Duration::from_millis(200);

pub(super) fn unexpected(what: &str, frame: &Frame) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("unexpected agent {what} reply: {frame:?}"),
    )
}

/// Sends `request` and hands every reply frame to `take` until `End`. An
/// `Err` frame fails (the serving side's "unsupported" as `Unsupported`), a
/// set `cancel` sends `Cancel` and stops with `Interrupted`. A dead transport
/// ends the wait through the connection's heartbeat, so no frame deadline is
/// needed (a host may hash one large file for a long time).
pub(super) fn stream(
    pool: &Arc<AgentPool>,
    request: Frame,
    cancel: &AtomicBool,
    mut take: impl FnMut(Frame) -> io::Result<()>,
) -> io::Result<()> {
    let lease = pool.lease();
    let mux = lease.mux()?;
    let (id, rx) = mux.register();
    let result = (|| {
        mux.send(id, request)?;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "agent operation canceled",
                ));
            }
            match rx.recv_timeout(CANCEL_POLL) {
                Ok(Frame::End) => return Ok(()),
                Ok(Frame::Err(error)) if is_unsupported(&error) => return Err(unsupported()),
                Ok(Frame::Err(error)) => return Err(agent_error(error)),
                // Keepalive of the service while the peer works.
                Ok(Frame::Progress { .. }) => {}
                Ok(frame) => take(frame)?,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "agent stream closed",
                    ))
                }
            }
        }
    })();
    if result.is_err() {
        let _ = mux.send(id, Frame::Cancel);
    }
    mux.unregister(id);
    result
}

pub(super) fn is_unsupported_error(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::Unsupported && is_unsupported(&error.to_string())
}
