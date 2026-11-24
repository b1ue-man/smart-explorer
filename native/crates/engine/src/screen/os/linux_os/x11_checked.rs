//! X11 requests without a reply whose errors are checked (round 12, Codex
//! 10): x11rb 0.13.2 hands the error of such a request, once its
//! `VoidCookie` is dropped, to the event queue ("Treat as events: Just drop
//! the cookie", `cookie.rs`), which no input connection reads, so a
//! remapping or a fake input the server refused counted as done. The
//! cookies of one run are checked together: the first `check` inserts one
//! sync with the server when no later request has a reply
//! (`RustConnection::check_for_raw_error`), which answers every earlier
//! request as well.
use std::io;

use x11rb::connection::RequestConnection;
use x11rb::cookie::VoidCookie;
use x11rb::errors::ConnectionError;

/// The requests sent since the last `check`.
pub(in crate::screen::os) struct Checked<'c, C: RequestConnection + ?Sized>(Vec<VoidCookie<'c, C>>);

impl<'c, C: RequestConnection + ?Sized> Checked<'c, C> {
    pub(in crate::screen::os) fn new() -> Self {
        Self(Vec::new())
    }

    /// Keeps the cookie of a request just sent; a request that could not be
    /// sent is the error.
    pub(in crate::screen::os) fn push(
        &mut self,
        sent: Result<VoidCookie<'c, C>, ConnectionError>,
    ) -> io::Result<()> {
        self.0.push(sent.map_err(io::Error::other)?);
        Ok(())
    }

    /// Whether the server took every request kept so far; the first error
    /// it names. The kept requests are settled either way.
    pub(in crate::screen::os) fn check(&mut self) -> io::Result<()> {
        let mut cookies = std::mem::take(&mut self.0).into_iter();
        while let Some(cookie) = cookies.next() {
            if let Err(error) = cookie.check() {
                cookies.for_each(VoidCookie::ignore_error);
                return Err(io::Error::other(error));
            }
        }
        Ok(())
    }
}

/// Requests left unchecked on an early return (an earlier error is being
/// reported) leave no errors in the unread event queue.
impl<C: RequestConnection + ?Sized> Drop for Checked<'_, C> {
    fn drop(&mut self) {
        self.0.drain(..).for_each(VoidCookie::ignore_error);
    }
}
