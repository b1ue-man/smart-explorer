//! Share-Server client side. The server is rendezvous-only and untrusted: it
//! routes signed presence for persistent direct contacts and rooms. File
//! operations run through persistent Iroh/QUIC sessions. Iroh attempts direct
//! peer-to-peer paths first and falls back to the configured relay while all
//! file frames remain end-to-end authenticated by the pinned relation.

#[path = "core/authorization_policy.rs"]
mod authorization_policy;
#[path = "core/backend.rs"]
mod backend;
#[path = "core/crypto.rs"]
mod core;
#[path = "core/exec.rs"]
mod exec;
#[path = "core/framing.rs"]
mod framing;
#[path = "core/fs.rs"]
mod fs;
#[path = "core/fs_error.rs"]
mod fs_error;
#[path = "core/identity.rs"]
mod identity;
#[path = "os/shared/identity_store.rs"]
mod identity_store;
#[path = "core/io_deadline.rs"]
mod io_deadline;
#[path = "core/line.rs"]
mod line;
#[path = "core/node.rs"]
mod node;
#[path = "core/peer_read.rs"]
mod peer_read;
#[path = "core/peer_writer.rs"]
mod peer_writer;
#[path = "core/profile_persistence.rs"]
mod profile_persistence;
#[path = "os/shared/profile_store.rs"]
mod profile_store;
#[path = "core/profiles.rs"]
mod profiles;
#[path = "core/server.rs"]
mod server;
#[path = "core/service.rs"]
mod service;
#[path = "core/session.rs"]
mod session;
#[path = "os/shared/system.rs"]
mod shared_system;
#[path = "core/signal_auth.rs"]
mod signal_auth;
#[path = "core/signal_connection.rs"]
mod signal_connection;
#[path = "core/signal_worker.rs"]
mod signal_worker;
#[cfg(windows)]
#[path = "os/windows/system.rs"]
mod system;
#[cfg(not(windows))]
#[path = "os/linux_os/system.rs"]
mod system;
#[path = "core/types.rs"]
mod types;
#[path = "core/walk.rs"]
mod walk;
#[path = "core/walk_assembly.rs"]
mod walk_assembly;
#[path = "core/wire.rs"]
mod wire;

pub use self::fs::{ShareExportConfig, SharedRoot};
pub use self::identity::{DirectCodeRotation, ShareIdentity};
pub use self::profile_persistence::ProfileChange;
pub use self::profiles::ShareProfiles;
pub use self::service::ShareService;
pub use self::types::{
    DirectAccessState, DirectGrantState, ExecRequest, ExecResult, PeerOpenTarget, PeerPresence,
    RoomMember, RoomProfile, ShareCmd, ShareEvent, ShareStatus,
};

pub fn core_now_secs() -> i64 {
    self::core::now_secs()
}

#[cfg(test)]
#[path = "core/backend_tests.rs"]
mod backend_tests;
#[cfg(test)]
#[path = "core/service_tests.rs"]
mod service_tests;
#[cfg(test)]
#[path = "core/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "core/walk_tests.rs"]
mod walk_tests;
