//! A request smb2 refused before it reached the wire for lack of credits.
//!
//! smb2 0.26 reserves a request's credits before it packs, signs or
//! encrypts it and registers the request as outstanding only afterwards
//! (`Inner::reserve_credits`, `execute_compound_inner`). A request that
//! finds too few credits while no other request is registered fails at once
//! with `CreditStarvation { waited: 0 }`, also when the credits are only held
//! by another request still being packed (a 1 MiB WRITE of a packet member
//! takes a while to sign): the 2026-10-28 measurement showed "needed 18, 15
//! available after waiting 0ns" for packet members against Samba. Nothing
//! was sent, so sending again is safe; it is sent again as soon as anything
//! can fund it: credits are on hand, or a request is outstanding whose
//! answer returns credits (then smb2 waits for them itself, under its own
//! deadline). A charge above the window the server stopped growing, or a
//! connection without credits and without anything in flight, stays an
//! error (errors.rs maps it to congestion, so the transfer backs off).
use smb2::client::Connection;
use smb2::Error;
use std::future::Future;

/// Whether `error` refused a request before the wire that may be sent again
/// right away on `conn`.
pub(super) fn passing(conn: &Connection, error: &Error) -> bool {
    let Error::CreditStarvation { needed, waited, .. } = error else {
        return false;
    };
    if !waited.is_zero() || conn.is_disconnected() {
        return false;
    }
    if conn
        .credit_ceiling()
        .is_some_and(|ceiling| *needed > ceiling)
    {
        return false;
    }
    conn.credits() >= *needed || !conn.outstanding_requests().is_empty()
}

/// `send` until it is not refused for a passing lack of credits.
pub(super) async fn funded<T, F, Fut>(conn: &Connection, send: F) -> smb2::Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = smb2::Result<T>>,
{
    loop {
        match send().await {
            Err(error) if passing(conn, &error) => tokio::task::yield_now().await,
            other => return other,
        }
    }
}
