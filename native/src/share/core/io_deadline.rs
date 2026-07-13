use std::future::Future;
use std::io;
use std::time::Duration;

use iroh::endpoint::{RecvStream, SendStream, VarInt};

pub(super) const PEER_OP_TIMEOUT: Duration = Duration::from_secs(60);
const PEER_ABORT_CODE: VarInt = VarInt::from_u32(1);

pub(super) fn abort(send: &mut SendStream, recv: &mut RecvStream) {
    let _ = recv.stop(PEER_ABORT_CODE);
    let _ = send.reset(PEER_ABORT_CODE);
}

pub(super) fn disconnected(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::NotConnected, error.to_string())
}

pub(super) async fn run<T, F>(operation: &str, future: F) -> io::Result<T>
where
    F: Future<Output = io::Result<T>>,
{
    run_for(operation, PEER_OP_TIMEOUT, future).await
}

pub(super) async fn run_for<T, F>(operation: &str, timeout: Duration, future: F) -> io::Result<T>
where
    F: Future<Output = io::Result<T>>,
{
    tokio::time::timeout(timeout, future).await.map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            format!("{operation} timed out after {} seconds", timeout.as_secs()),
        )
    })?
}

pub(super) async fn run_until<T, F>(
    deadline: tokio::time::Instant,
    timeout_message: &'static str,
    future: F,
) -> io::Result<T>
where
    F: Future<Output = io::Result<T>>,
{
    tokio::time::timeout_at(deadline, future)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, timeout_message))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_returns_values_and_explicit_timeouts() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let value = runtime
            .block_on(run_for("test", Duration::from_secs(1), async {
                Ok::<_, io::Error>(7)
            }))
            .unwrap();
        assert_eq!(value, 7);

        let error = runtime
            .block_on(run_for("test", Duration::from_millis(1), async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok::<_, io::Error>(())
            }))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("test"));

        let error = disconnected("peer closed");
        assert_eq!(error.kind(), io::ErrorKind::NotConnected);

        let error = runtime
            .block_on(run_until(
                tokio::time::Instant::now() + Duration::from_millis(1),
                "absolute timeout",
                std::future::pending::<io::Result<()>>(),
            ))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_eq!(error.to_string(), "absolute timeout");
    }
}
