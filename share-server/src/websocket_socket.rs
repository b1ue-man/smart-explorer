//! Readiness and outbound wakes for the synchronous ws/wss owner thread.
//! One process-wide I/O reactor replaces periodic connection polling.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use tokio::runtime::Runtime;
use tokio::sync::Notify;

static REACTOR: OnceLock<Result<Runtime, String>> = OnceLock::new();

pub(super) struct SocketIo {
    socket: TcpStream,
    registered: tokio::net::TcpStream,
    reactor: &'static Runtime,
    notify: Arc<Notify>,
    write_deadline: Option<Instant>,
}

impl SocketIo {
    pub(super) fn new(socket: TcpStream) -> io::Result<Self> {
        let reactor = REACTOR
            .get_or_init(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(1)
                    .enable_all()
                    .thread_name("share-server-io")
                    .build()
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(|error| io::Error::other(error.clone()))?;
        socket.set_nonblocking(true)?;
        let registered = {
            let _entered = reactor.enter();
            tokio::net::TcpStream::from_std(socket.try_clone()?)?
        };
        Ok(Self {
            socket,
            registered,
            reactor,
            notify: Arc::new(Notify::new()),
            write_deadline: None,
        })
    }

    pub(super) fn socket(&self) -> &TcpStream {
        &self.socket
    }
    pub(super) fn notify(&self) -> Arc<Notify> {
        self.notify.clone()
    }
    pub(super) fn set_write_deadline(&mut self, deadline: Option<Instant>) {
        self.write_deadline = deadline;
    }

    pub(super) fn wait_readable(&self, deadline: Instant) -> io::Result<()> {
        self.reactor.block_on(async {
            tokio::select! {
                ready = self.registered.readable() => ready,
                _ = self.notify.notified() => Ok(()),
                _ = tokio::time::sleep_until(deadline.into()) =>
                    Err(io::Error::new(io::ErrorKind::TimedOut, "signaling read deadline expired")),
            }
        })
    }
}

impl Read for SocketIo {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        // try_read clears readiness on WouldBlock; readable() is therefore
        // safe after a partial TLS record or a fragmented WebSocket frame.
        self.registered.try_read(bytes)
    }
}

impl Write for SocketIo {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let timeout = Instant::now() + crate::writer::WRITE_TIMEOUT;
        let deadline = self
            .write_deadline
            .map_or(timeout, |deadline| deadline.min(timeout));
        loop {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "signaling write deadline expired",
                ));
            }
            match self.registered.try_write(bytes) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    self.reactor
                        .block_on(async {
                            tokio::time::timeout_at(deadline.into(), self.registered.writable())
                                .await
                        })
                        .map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::TimedOut,
                                "signaling write deadline expired",
                            )
                        })??;
                }
                result => return result,
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;
    use crate::{Out, Writer};

    #[test]
    fn review_task_queued_output_wakes_readiness_without_socket_input() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let mut socket = SocketIo::new(listener.accept().unwrap().0).unwrap();
        assert_eq!(
            socket.read(&mut [0]).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        let (writer, outbound) = Writer::test_raw_channel(1);
        writer.set_wake(socket.notify());
        let (ready_send, ready) = mpsc::channel();
        let (done_send, done) = mpsc::channel();
        let wait = std::thread::spawn(move || {
            ready_send.send(()).unwrap();
            socket
                .wait_readable(Instant::now() + Duration::from_secs(10))
                .unwrap();
            let message: serde_json::Value =
                serde_json::from_slice(outbound.try_recv().unwrap().json()).unwrap();
            done_send.send(message).unwrap();
        });
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(writer.try_send(&Out::Pong));
        // A wake does not depend on inbound traffic or the ten-second timer.
        assert_eq!(
            done.recv_timeout(Duration::from_secs(2)).unwrap()["t"],
            "pong"
        );
        wait.join().unwrap();
    }
}
