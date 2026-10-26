//! Two bounded copy directions. EOF shuts down only the opposite writer;
//! the remaining direction drains independently. Route pauses preserve the
//! same pending operation and pause its active idle budget (P28).
use crate::share::tcp_forward::TCP_COPY_BYTES;
use crate::share::tcp_registry::TcpCounters;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use std::{
    io,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::watch;

const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

pub(crate) async fn pump(
    connection: &Connection,
    socket: &mut tokio::net::TcpStream,
    send: &mut SendStream,
    recv: &mut RecvStream,
    counters: Arc<TcpCounters>,
) -> io::Result<()> {
    counters.active.fetch_add(1, Ordering::Relaxed);
    struct Active(Arc<TcpCounters>);
    impl Drop for Active {
        fn drop(&mut self) {
            self.0.active.fetch_sub(1, Ordering::Relaxed);
        }
    }
    let _active = Active(counters.clone());
    let (mut read, mut write) = socket.split();
    let (progress, changes) = watch::channel(crate::share::route_protocol::active_now());
    let transfer = async {
        tokio::try_join!(
            copy(connection, &mut read, send, &counters.read, &progress),
            copy(connection, recv, &mut write, &counters.written, &progress)
        )?;
        Ok(())
    };
    tokio::select! {
        result = transfer => result,
        error = idle(changes) => Err(error),
    }
}
async fn copy<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    connection: &Connection,
    read: &mut R,
    write: &mut W,
    count: &std::sync::atomic::AtomicU64,
    progress: &watch::Sender<tokio::time::Instant>,
) -> io::Result<()> {
    let mut bytes = [0; TCP_COPY_BYTES];
    loop {
        // The shared activity monitor owns idle expiry. A quiet read half
        // must not terminate a still-progressing one-way upload/download.
        let length = crate::share::route_protocol::run_with_route(
            connection,
            Duration::from_secs(86400),
            None,
            read.read(&mut bytes),
        )
        .await?;
        if length == 0 {
            crate::share::route_protocol::run_with_route(
                connection,
                Duration::from_secs(86400),
                None,
                write.shutdown(),
            )
            .await?;
            progress.send_replace(crate::share::route_protocol::active_now());
            return Ok(());
        }
        crate::share::route_protocol::run_with_route(
            connection,
            Duration::from_secs(86400),
            None,
            write.write_all(&bytes[..length]),
        )
        .await?;
        count.fetch_add(length as u64, Ordering::Relaxed);
        progress.send_replace(crate::share::route_protocol::active_now());
    }
}
async fn idle(mut progress: watch::Receiver<tokio::time::Instant>) -> io::Error {
    loop {
        let deadline = *progress.borrow_and_update() + IDLE_TIMEOUT;
        tokio::select! {
            result = progress.changed() => if result.is_err() { return io::Error::other("TCP pump stopped"); },
            () = crate::share::route_protocol::sleep_until_active(deadline) => {
                return io::Error::new(io::ErrorKind::TimedOut, "TCP connection idle for 120 active seconds");
            }
        }
    }
}
