//! One exec request whose output is awaited for a bounded time and can be
//! canceled: the hash command of `digest_command.rs`. Right after the
//! request its standard input gets the path and then EOF (RFC 4254 section
//! 5.3 `SSH_MSG_CHANNEL_EOF`), so a command that reads input (the script, or
//! an SFTP server a forced command put there) never waits for more. A cancel
//! sends the RFC 4254 section 6.9 signal `TERM` and closes the channel;
//! running past the wait closes it too (the server ends the command with its
//! channel).
use super::backend::SftpBackend;
use super::exec::{absorb, CapturedExec, ChannelOpen};
use russh::{client, Sig};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How often a waiting call looks at its cancel flag.
const POLL: Duration = Duration::from_millis(200);

/// Runs `command` on a new session channel of the connection, sends `stdin`
/// and EOF, and returns what it printed. `Ok(None)`: the server refused one
/// more channel while no other digest command ran on this connection; a
/// refusal while others run waits for one of them to end (exec_slots.rs).
pub(super) fn run(
    backend: &SftpBackend,
    command: &str,
    stdin: &[u8],
    wait: Duration,
    cancel: &AtomicBool,
) -> io::Result<Option<CapturedExec>> {
    if cancel.load(Ordering::Relaxed) {
        return Err(canceled());
    }
    let mut slot = backend.exec_slots.enter(cancel).ok_or_else(canceled)?;
    let (generation, mut channel) = loop {
        match backend.try_open_session_channel()? {
            ChannelOpen::Opened(generation, channel) => break (generation, channel),
            ChannelOpen::Refused(_) if backend.exec_slots.refused(&slot) => {
                drop(slot);
                slot = backend.exec_slots.enter(cancel).ok_or_else(canceled)?;
            }
            ChannelOpen::Refused(_) => return Ok(None),
        }
    };
    // The slot stays taken until the command ended.
    let _running = slot;
    backend.request_exec(&generation, &channel, true, command)?;
    let captured = backend.rt.block_on(async {
        feed(&channel, stdin).await;
        capture(&mut channel, wait, cancel).await
    });
    if generation.session().is_closed() {
        backend.connection.mark_stale(&generation);
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "SSH transport closed while waiting for the hash command",
        ));
    }
    captured.map(Some)
}

/// Sends `stdin` and EOF. A failed send is not reported here: the channel's
/// own messages (a rejected request, a close) tell `capture` what happened.
pub(super) async fn feed(channel: &russh::Channel<client::Msg>, stdin: &[u8]) {
    if channel.data(stdin).await.is_ok() {
        let _ = channel.eof().await;
    }
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
