//! Waits for incoming signal data without polling the connection.
//!
//! A watcher thread blocks in `peek` on a second handle of the socket and
//! reports when bytes (or the end of the stream) arrive. The worker then
//! reads everything available without blocking (including what its line
//! or TLS buffers already hold) and re-arms the watcher. `peek` consumes
//! nothing, so the worker keeps exclusive ownership of the TLS and
//! WebSocket state, and the strict alternation means the watcher never
//! waits while the worker reads.

use std::io;
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{bounded, Receiver, Sender};

use crate::share::signal_connection::SignalConnection;

/// Socket read timeout while the watcher waits; it only bounds how long a
/// stopped watcher may linger where a shutdown does not wake it.
pub(super) const SIGNAL_WATCH_TIMEOUT: Duration = Duration::from_secs(300);

pub(super) struct SignalReadiness {
    ready: Receiver<()>,
    rearm: Sender<()>,
    stop: Arc<AtomicBool>,
    control: TcpStream,
}

impl SignalReadiness {
    pub(super) fn attach(connection: &mut SignalConnection) -> io::Result<Self> {
        let watched = connection.watch_handle()?;
        let control = connection.watch_handle()?;
        // Set on the watcher's handle too: where a platform keeps socket
        // timeouts per handle, the clone must not poll at the short timeout.
        watched.set_read_timeout(Some(SIGNAL_WATCH_TIMEOUT))?;
        let (ready_sender, ready) = bounded(1);
        let (rearm, rearm_receiver) = bounded(1);
        let stop = Arc::new(AtomicBool::new(false));
        let watcher_stop = stop.clone();
        let spawned = std::thread::Builder::new()
            .name("share-signal-rd".into())
            .spawn(move || watch(watched, ready_sender, rearm_receiver, watcher_stop));
        if let Err(error) = spawned {
            // The session polls instead; undo the shared socket timeout.
            let resting = connection.resting_read_timeout();
            connection.rest_reads_for(resting);
            return Err(error);
        }
        connection.rest_reads_for(SIGNAL_WATCH_TIMEOUT);
        Ok(Self {
            ready,
            rearm,
            stop,
            control,
        })
    }

    /// Fires once data, the end of the stream or an error is pending.
    pub(super) fn ready(&self) -> &Receiver<()> {
        &self.ready
    }

    /// The worker drained the socket (the first time right after
    /// attaching); wait for the next data.
    pub(super) fn rearm(&self) {
        let _ = self.rearm.try_send(());
    }
}

impl Drop for SignalReadiness {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // The connection ends with its readiness; this wakes the watcher.
        let _ = self.control.shutdown(Shutdown::Both);
    }
}

fn watch(socket: TcpStream, ready: Sender<()>, rearm: Receiver<()>, stop: Arc<AtomicBool>) {
    let mut probe = [0u8; 1];
    // Armed by the worker only after it read what the handshake left
    // buffered; until it re-arms, the socket belongs to the worker alone.
    while rearm.recv().is_ok() {
        loop {
            if stop.load(Ordering::Acquire) {
                return;
            }
            match socket.peek(&mut probe) {
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) => {}
                // Data, end of stream or an error: the worker reads and
                // learns which.
                _ => break,
            }
        }
        if ready.send(()).is_err() {
            return;
        }
    }
}
