//! One exec request whose output is awaited for a bounded time and can be
//! canceled: the hash command of `digest_command.rs`. A cancel sends the
//! RFC 4254 section 6.9 signal `TERM` and closes the channel; running past
//! the wait closes it too (the server ends the command with its channel).
use super::backend::SftpBackend;
use super::exec::{absorb, CapturedExec, ChannelOpen};
use russh::{client, Sig};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How often a waiting call looks at its cancel flag.
const POLL: Duration = Duration::from_millis(200);

/// Runs `command` on a new session channel of the connection and returns what
/// it printed. `Ok(None)`: the server refused one more channel.
pub(super) fn run(
    backend: &SftpBackend,
    command: &str,
    wait: Duration,
    cancel: &AtomicBool,
) -> io::Result<Option<CapturedExec>> {
    if cancel.load(Ordering::Relaxed) {
        return Err(canceled());
    }
    let (generation, mut channel) = match backend.try_open_session_channel()? {
        ChannelOpen::Opened(generation, channel) => (generation, channel),
        ChannelOpen::Refused(_) => return Ok(None),
    };
    backend.request_exec(&generation, &channel, true, command)?;
    let captured = backend.rt.block_on(capture(&mut channel, wait, cancel));
    if generation.session().is_closed() {
        backend.connection.mark_stale(&generation);
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "SSH transport closed while waiting for the hash command",
        ));
    }
    captured.map(Some)
}

/// Collects the channel until it ends, `wait` has passed or `cancel` is set.
pub(super) async fn capture(
    channel: &mut russh::Channel<client::Msg>,
    wait: Duration,
    cancel: &AtomicBool,
) -> io::Result<CapturedExec> {
    let started = Instant::now();
    let mut capture = CapturedExec::default();
    loop {
        if cancel.load(Ordering::Relaxed) {
            end(channel, true).await;
            return Err(canceled());
        }
        if started.elapsed() >= wait {
            end(channel, true).await;
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the hash command did not finish in time",
            ));
        }
        match tokio::time::timeout(POLL, channel.wait()).await {
            Err(_) => continue,
            Ok(message) => {
                if !absorb(&mut capture, message)? {
                    return Ok(capture);
                }
            }
        }
    }
}

async fn end(channel: &mut russh::Channel<client::Msg>, terminate: bool) {
    if terminate {
        let _ = channel.signal(Sig::TERM).await;
    }
    let _ = channel.close().await;
}

fn canceled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "content digest canceled")
}
