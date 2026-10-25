//! Owns the retained worker from connection admission through actual client completion.
use super::*;

pub(crate) fn spawn_connected(connection: Connection, endpoint: PeerEndpoint,
    identity: ShareIdentity, start: ExecStart) -> ExecClientSession {
    spawn_connected_retained(connection, endpoint, identity, start, None)
}

pub(crate) fn spawn_connected_retained(connection: Connection, endpoint: PeerEndpoint,
    identity: ShareIdentity, start: ExecStart, retainer: Option<std::sync::Arc<dyn Send + Sync>>) -> ExecClientSession {
    spawn_connected_cancellable(connection, endpoint, identity, start, retainer, None)
}

pub(crate) fn spawn_connected_cancellable(connection: Connection, endpoint: PeerEndpoint,
    identity: ShareIdentity, start: ExecStart, retainer: Option<std::sync::Arc<dyn Send + Sync>>,
    cancellation: Option<std::sync::Arc<crate::share::route_cancellation::RouteCancellation>>) -> ExecClientSession {
    let exec_id = start.exec_id.clone();
    let (input_tx, input_rx) = mpsc::channel(INPUT_CAPACITY);
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (event_tx, event_rx) = mpsc::channel(EVENT_CAPACITY);
    let (notice_tx, notice_rx) = watch::channel(ExecClientNotice::default());
    let task = tokio::spawn(async move {
        let _retainer = retainer;
        if let Some(cancellation) = &cancellation {
            cancellation.bind(&connection).map_err(|error| classify_io(error, false, "setup_cancelled"))?;
        }
        let cancel_connection = connection.clone();
        let run = run_connected(connection, endpoint, identity, start, input_rx, cancel_rx, event_tx, notice_tx);
        tokio::pin!(run);
        let cancelled = async {
            if let Some(cancellation) = &cancellation { cancellation.cancelled().await; }
            else { std::future::pending::<()>().await; }
        };
        tokio::select! {
            biased;
            _ = cancelled => {
                if let Some(cancellation) = &cancellation { cancellation.cancel(); }
                cancel_connection.close(VarInt::from_u32(0x4558), b"Exec setup cancelled");
                // Keep the retainer while the existing worker observes transport closure.
                run.await
            }
            result = &mut run => result,
        }
    });
    ExecClientSession { exec_id, input: ExecInputSender::new(input_tx, cancel_tx), events: event_rx, task, notice: notice_rx }
}
