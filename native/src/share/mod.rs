//! Share-Server client side. The server is rendezvous-only and untrusted: it
//! routes signed presence for persistent direct contacts and rooms. File
//! operations run directly peer-to-peer over Noise XXpsk3 channels whose static
//! keys are pinned by direct/room relation secrets.

#[path = "core/backend.rs"]
mod backend;
#[path = "core/crypto.rs"]
mod core;
#[path = "core/fs.rs"]
mod fs;
#[path = "core/identity.rs"]
mod identity;
#[path = "core/profiles.rs"]
mod profiles;
#[path = "core/protocol.rs"]
mod protocol;
#[path = "core/service.rs"]
mod service;
#[path = "os/shared/system.rs"]
mod system;
#[path = "os/shared/transfer.rs"]
mod transfer;
#[path = "core/types.rs"]
mod types;
#[path = "core/wire.rs"]
mod wire;

pub use self::fs::{ShareExportConfig, SharedRoot};
pub use self::identity::ShareIdentity;
pub use self::profiles::ShareProfiles;
pub use self::service::ShareService;
pub use self::types::{
    PeerOpenTarget, PeerPresence, RoomMember, RoomProfile, ShareCmd, ShareEvent, ShareStatus,
};

pub fn core_now_secs() -> i64 {
    self::core::now_secs()
}

pub fn local_lan_ips() -> Vec<String> {
    self::system::lan_ips()
}

#[cfg(test)]
#[path = "core/tests.rs"]
mod tests;
