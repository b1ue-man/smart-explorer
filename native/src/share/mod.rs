//! Share-Server client side. The server is rendezvous-only and untrusted: it
//! routes signed presence for persistent direct contacts and rooms. File
//! operations run through persistent Iroh/QUIC sessions. Iroh attempts direct
//! peer-to-peer paths first and falls back to the configured relay while all
//! file frames remain end-to-end authenticated by the pinned relation.

#[path = "core/backend.rs"]
mod backend;
#[path = "core/crypto.rs"]
mod core;
#[path = "core/fs.rs"]
mod fs;
#[path = "core/identity.rs"]
mod identity;
#[path = "core/line.rs"]
mod line;
#[path = "core/profiles.rs"]
mod profiles;
#[path = "core/service.rs"]
mod service;
#[path = "os/shared/system.rs"]
mod shared_system;
#[cfg(windows)]
#[path = "os/windows/system.rs"]
mod system;
#[cfg(not(windows))]
#[path = "os/linux_os/system.rs"]
mod system;
#[path = "core/types.rs"]
mod types;
#[path = "core/wire.rs"]
mod wire;

pub use self::fs::{ShareExportConfig, SharedRoot};
pub use self::identity::ShareIdentity;
pub use self::profiles::ShareProfiles;
pub use self::service::ShareService;
pub use self::types::{
    DirectAccessState, DirectGrantState, PeerOpenTarget, PeerPresence, RoomMember, RoomProfile,
    ShareCmd, ShareEvent, ShareStatus,
};

pub fn core_now_secs() -> i64 {
    self::core::now_secs()
}

#[cfg(test)]
#[path = "core/tests.rs"]
mod tests;
