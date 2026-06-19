//! Peer file sharing (#21) - client side. The maintainer's server only *routes
//! discovery* (see `share-server/`); here we connect to it, learn a peer's
//! reachable candidates, then open a **direct** TCP connection and transfer
//! files **end-to-end encrypted** with Noise `NNpsk0` keyed by a PSK derived
//! from the shared pairing/room **code**. The server never sees file bytes.
//!
//! Two modes: **pair** (two devices, one code) and **room** (many devices, one
//! code, share to all). The GUI drives this through `ShareCmd`/`ShareEvent`.
//!
//! NOTE: the live networked path (NAT traversal, Noise handshake, transfer)
//! cannot be exercised in the headless build env; it compiles for host +
//! windows-gnu and the pure logic is unit-tested. Needs a real two-machine test.

mod advertise;
mod core;
mod core_oslocked;
mod protocol;
mod service;
mod session;
mod transfer;
mod types;
mod wire;

pub use self::core::gen_code;
pub use self::service::ShareService;
pub use self::types::{RemoteDevice, ShareCmd, ShareEvent};

#[cfg(test)]
mod tests;
