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

#[path = "core/advertise.rs"]
mod advertise;
#[path = "core/backend.rs"]
mod backend;
#[path = "core/crypto.rs"]
mod core;
#[path = "core/fs.rs"]
mod fs;
#[path = "core/protocol.rs"]
mod protocol;
#[path = "core/service.rs"]
mod service;
#[path = "core/session.rs"]
mod session;
#[path = "os/shared/system.rs"]
mod system;
#[path = "os/shared/transfer.rs"]
mod transfer;
#[path = "core/types.rs"]
mod types;
#[path = "core/wire.rs"]
mod wire;

pub use self::core::gen_code;
pub use self::fs::{ShareExportConfig, SharedRoot};
pub use self::service::ShareService;
pub use self::types::{RemoteDevice, ShareCmd, ShareEvent};

#[cfg(test)]
#[path = "core/tests.rs"]
mod tests;
