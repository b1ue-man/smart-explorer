//! The host's answer to a hello it rejects (R3, A9; plan 2026-11-06).
use std::io;

use iroh::endpoint::{Connection, SendStream};

use super::super::framing::send_ctrl;
use super::super::wire::{Ctrl, FsResponse};

/// Sends a hello rejection so the client can read it. Dropping the last
/// handle closes the connection at once, and a peer that receives
/// CONNECTION_CLOSE may drop stream data its application has not read
/// (noq `Connection::close`): the client would see a lost connection instead
/// of the reason (R3) and would not pause its retries (A9). The client closes
/// after reading the rejection; the handshake deadline bounds the wait, as
/// for Exec (`exec_admission.rs`).
pub(super) async fn refuse_hello(
    conn: &Connection,
    send: &mut SendStream,
    deadline: tokio::time::Instant,
    resp: FsResponse,
) -> io::Result<()> {
    super::super::route_protocol::run_until_active(
        deadline,
        "Session-Ablehnung Timeout",
        send_ctrl(send, &Ctrl::FsResp { resp }),
    )
    .await?;
    let _ = send.finish();
    let _ = super::super::route_protocol::timeout_until(deadline, conn.closed()).await;
    Ok(())
}
