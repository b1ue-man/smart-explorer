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
#[path = "os/shared/direct_actions.rs"]
mod direct_actions;
#[path = "core/direct_ledger.rs"]
mod direct_ledger;
#[path = "core/direct_ledger_mutations.rs"]
mod direct_ledger_mutations;
#[path = "core/direct_ledger_retention.rs"]
mod direct_ledger_retention;
#[path = "core/direct_ledger_validation.rs"]
mod direct_ledger_validation;
#[path = "core/direct_lifecycle.rs"]
mod direct_lifecycle;
#[path = "core/direct_lifecycle_error.rs"]
mod direct_lifecycle_error;
#[path = "core/direct_messages.rs"]
mod direct_messages;
#[path = "core/direct_protocol.rs"]
mod direct_protocol;
#[path = "core/direct_signal_event.rs"]
mod direct_signal_event;
#[path = "core/direct_transcript.rs"]
mod direct_transcript;
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
#[cfg(not(windows))]
#[path = "os/linux_os/identity_lock.rs"]
mod identity_lock;
#[cfg(windows)]
#[path = "os/windows/identity_lock.rs"]
mod identity_lock;
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
#[path = "core/signal_commands.rs"]
mod signal_commands;
#[path = "core/signal_connection.rs"]
mod signal_connection;
#[path = "core/signal_connector.rs"]
mod signal_connector;
#[path = "core/signal_handshake.rs"]
mod signal_handshake;
#[path = "core/signal_worker.rs"]
mod signal_worker;
#[cfg(windows)]
#[path = "os/windows/system.rs"]
mod system;
#[cfg(not(windows))]
#[path = "os/linux_os/system.rs"]
mod system;
#[path = "core/tracked_signal_dispatch.rs"]
mod tracked_signal_dispatch;
#[path = "core/tracked_signal_outbox.rs"]
mod tracked_signal_outbox;
#[path = "core/tracked_signal_sender.rs"]
mod tracked_signal_sender;
#[path = "core/tracked_signal_verify.rs"]
mod tracked_signal_verify;
#[path = "core/types.rs"]
mod types;
#[path = "core/walk.rs"]
mod walk;
#[path = "core/walk_assembly.rs"]
mod walk_assembly;
#[path = "core/wire.rs"]
mod wire;

pub use self::direct_actions::{
    decide_direct_request, queue_direct_request_for_contact, retry_direct_request_now,
    DirectRequestAction,
};
pub use self::direct_ledger::{
    DirectEnvelopeKind, DirectLedgerError, DirectRelayOutcome, DirectRequestDirection,
    DirectRequestEntry, DirectRequestRetries, DirectRetryState, MAX_DIRECT_REQUEST_ENTRIES,
};
pub use self::direct_lifecycle::{
    DirectDecisionDeliveryState, DirectDecisionDeliveryStatus, DirectDecisionState,
    DirectDecisionStatus, DirectDeliveryState, DirectDeliveryStatus, DirectFailure,
    DirectLifecycleEvent, DirectRequestRecord,
};
pub use self::direct_lifecycle_error::DirectLifecycleError;
pub use self::direct_protocol::{
    DirectDecisionKind, DirectPeerIdentity, DirectProtocolError, DirectRequestId,
    SignedDirectDecision, SignedDirectDecisionReceipt, SignedDirectRequest,
    SignedDirectRequestReceipt,
};
pub use self::direct_signal_event::DirectSignalEvent;
pub use self::fs::{ShareExportConfig, SharedRoot};
pub use self::identity::{DirectCodeRotation, IdentityRepair, IdentityRepairAction, ShareIdentity};
pub use self::profile_persistence::ProfileChange;
pub(crate) use self::profiles::ProfileRevision;
pub use self::profiles::ShareProfiles;
pub use self::service::ShareService;
pub use self::types::{
    DirectAccessState, DirectContact, DirectGrantState, ExecRequest, ExecResult, PeerOpenTarget,
    PeerPresence, RoomMember, RoomProfile, ShareCmd, ShareEvent, ShareStatus,
};

pub fn core_now_secs() -> i64 {
    self::core::now_secs()
}

#[cfg(test)]
#[path = "core/backend_tests.rs"]
mod backend_tests;
#[cfg(test)]
#[path = "core/direct_ledger_retention_tests.rs"]
mod direct_ledger_retention_tests;
#[cfg(test)]
#[path = "core/direct_ledger_tests.rs"]
mod direct_ledger_tests;
#[cfg(test)]
#[path = "core/direct_lifecycle_tests.rs"]
mod direct_lifecycle_tests;
#[cfg(test)]
#[path = "core/direct_protocol_tests.rs"]
mod direct_protocol_tests;
#[cfg(test)]
#[path = "core/service_tests.rs"]
mod service_tests;
#[cfg(test)]
#[path = "core/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "core/tracked_signal_tests.rs"]
mod tracked_signal_tests;
#[cfg(test)]
#[path = "core/walk_tests.rs"]
mod walk_tests;
