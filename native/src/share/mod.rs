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

#[path = "core_oslocked/advertise.rs"]
mod advertise;
#[path = "core/crypto.rs"]
mod core;
#[path = "core_oslocked/system.rs"]
mod core_oslocked;
#[path = "core_oslocked/protocol.rs"]
mod protocol;
#[path = "core_oslocked/service.rs"]
mod service;
#[path = "core_oslocked/session.rs"]
mod session;
#[path = "core_oslocked/transfer.rs"]
mod transfer;
#[path = "core/types.rs"]
mod types;
#[path = "core/wire.rs"]
mod wire;

pub use self::core::gen_code;
pub use self::service::ShareService;
pub use self::types::{RemoteDevice, ShareCmd, ShareEvent};

#[cfg(test)]
#[path = "core/tests.rs"]
mod tests;
