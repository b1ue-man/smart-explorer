//! What the session needs of the platform for the host's sound (plan
//! 2026-11-17 P9 „Audio“): a host backend that can capture its output
//! names an `AudioHost` (`ScreenHostBackend::audio`); one that cannot names
//! none, and the session then never answers a sound offer. The session
//! reaches the platform only through this port, like the picture through
//! `ScreenHostBackend`.
use iroh::endpoint::Connection;

/// The platform's sound link of the host.
pub trait AudioHost: Send + Sync {
    /// Attaches the link to a session connection after `HELLO_OK` with
    /// `audio`; it sends nothing before `set_wanted(true)` and ends with
    /// the connection.
    fn attach(&self, connection: &Connection);
    /// The viewer's `AUDIO`: the sound on or off.
    fn set_wanted(&self, connection: &Connection, on: bool);
}
