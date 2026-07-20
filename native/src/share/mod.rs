//! Share-Server client side. The server is rendezvous-only and untrusted: it
//! routes signed presence for persistent direct contacts and rooms. File
//! operations run through persistent Iroh/QUIC sessions. Iroh attempts direct
//! peer-to-peer paths first and falls back to the configured relay while all
//! file frames remain end-to-end authenticated by the pinned relation.

#[path = "core/authorization_policy.rs"]
mod authorization_policy;
#[path = "core/backend.rs"]
mod backend;
#[path = "core/blocking.rs"]
mod blocking;
#[path = "core/connection_events.rs"]
mod connection_events;
#[path = "core/crypto.rs"]
mod core;
#[path = "os/shared/direct_actions.rs"]
mod direct_actions;
#[cfg(test)]
#[path = "core/direct_identity_conflict_tests.rs"]
mod direct_identity_conflict_tests;
#[path = "core/direct_ledger.rs"]
mod direct_ledger;
#[path = "core/direct_ledger_mutations.rs"]
mod direct_ledger_mutations;
#[path = "core/direct_ledger_projection.rs"]
mod direct_ledger_projection;
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
#[path = "core/direct_request_tombstone.rs"]
mod direct_request_tombstone;
#[path = "core/direct_signal_event.rs"]
mod direct_signal_event;
#[path = "core/direct_transcript.rs"]
mod direct_transcript;
#[path = "core/endpoint_routes.rs"]
mod endpoint_routes;
#[path = "core/exec.rs"]
mod exec;
#[path = "core/exec_auth.rs"]
mod exec_auth;
#[path = "core/exec_client.rs"]
mod exec_client;
#[path = "core/exec_client_active.rs"]
mod exec_client_active;
#[path = "core/exec_frame_reader.rs"]
mod exec_frame_reader;
#[path = "core/exec_grant_runtime.rs"]
mod exec_grant_runtime;
#[path = "core/exec_heartbeat.rs"]
mod exec_heartbeat;
#[path = "core/exec_job.rs"]
mod exec_job;
#[path = "core/exec_platform.rs"]
mod exec_platform;
#[path = "core/exec_policy.rs"]
mod exec_policy;
#[path = "core/exec_protocol.rs"]
mod exec_protocol;
#[path = "core/exec_registry.rs"]
mod exec_registry;
#[path = "core/exec_server.rs"]
mod exec_server;
#[path = "core/exec_session.rs"]
mod exec_session;
#[path = "core/exec_supervisor_protocol.rs"]
mod exec_supervisor_protocol;
#[path = "core/exec_types.rs"]
mod exec_types;
#[path = "core/framing.rs"]
mod framing;
#[path = "core/fs.rs"]
mod fs;
#[path = "core/fs_error.rs"]
mod fs_error;
#[path = "core/handshake_limits.rs"]
mod handshake_limits;
#[path = "core/identity.rs"]
mod identity;
#[cfg(not(windows))]
#[path = "os/linux_os/identity_lock.rs"]
mod identity_lock;
#[cfg(windows)]
#[path = "os/windows/identity_lock.rs"]
mod identity_lock;
#[cfg(test)]
#[path = "core/identity_profile_reconciliation_tests.rs"]
mod identity_profile_reconciliation_tests;
#[path = "os/shared/identity_store.rs"]
mod identity_store;
#[path = "core/io_deadline.rs"]
mod io_deadline;
#[path = "core/keepalive.rs"]
mod keepalive;
#[path = "os/shared/legacy_direct_actions.rs"]
mod legacy_direct_actions;
#[path = "core/legacy_direct_request.rs"]
mod legacy_direct_request;
#[path = "core/legacy_direct_request_mutations.rs"]
mod legacy_direct_request_mutations;
#[path = "core/legacy_direct_request_reconciliation.rs"]
mod legacy_direct_request_reconciliation;
#[cfg(test)]
#[path = "core/legacy_direct_request_tests.rs"]
mod legacy_direct_request_tests;
#[path = "core/legacy_direct_request_validation.rs"]
mod legacy_direct_request_validation;
#[path = "core/line.rs"]
mod line;
#[path = "core/node.rs"]
mod node;
#[path = "core/peer_read.rs"]
mod peer_read;
#[path = "core/peer_writer.rs"]
mod peer_writer;
#[cfg(target_os = "linux")]
#[path = "os/linux_os/exec.rs"]
mod platform_exec;
#[cfg(windows)]
#[path = "os/windows/exec.rs"]
mod platform_exec;
#[path = "os/shared/profile_operations.rs"]
mod profile_operations;
#[path = "core/profile_persistence.rs"]
mod profile_persistence;
#[path = "os/shared/profile_store.rs"]
mod profile_store;
#[path = "core/profiles.rs"]
mod profiles;
#[path = "core/server.rs"]
mod server;
#[path = "core/server_transfer.rs"]
mod server_transfer;
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
#[cfg(test)]
#[path = "core/signal_configure_tests.rs"]
mod signal_configure_tests;
#[path = "core/signal_connection.rs"]
mod signal_connection;
#[path = "core/signal_connector.rs"]
mod signal_connector;
#[path = "core/signal_handshake.rs"]
mod signal_handshake;
#[path = "core/signal_presence.rs"]
mod signal_presence;
#[path = "core/signal_subscriptions.rs"]
mod signal_subscriptions;
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
#[cfg(test)]
#[path = "core/tracked_signal_sender_tests.rs"]
mod tracked_signal_sender_tests;
#[path = "core/tracked_signal_verify.rs"]
mod tracked_signal_verify;
#[path = "os/shared/transport_options.rs"]
mod transport_options;
#[path = "core/types.rs"]
mod types;
#[path = "core/walk.rs"]
mod walk;
#[path = "core/walk_assembly.rs"]
mod walk_assembly;
#[path = "core/wire.rs"]
mod wire;

pub use self::direct_actions::{
    decide_direct_request, delete_direct_request_history, queue_direct_request_for_contact,
    retry_direct_request_now, DirectRequestAction,
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
pub(crate) use self::direct_protocol::MAX_TRACKED_DIRECT_ENVELOPE_LIFETIME_SECS;
pub use self::direct_protocol::{
    DirectDecisionKind, DirectPeerIdentity, DirectProtocolError, DirectRequestId,
    SignedDirectDecision, SignedDirectDecisionReceipt, SignedDirectRequest,
    SignedDirectRequestReceipt,
};
pub use self::direct_request_tombstone::DirectRequestTombstone;
pub use self::direct_signal_event::DirectSignalEvent;
pub(crate) use self::exec_client::{ExecClientEvent, ExecClientInput};
pub use self::exec_grant_runtime::ExecGrantMutation;
pub use self::exec_policy::ExecGrant;
pub(crate) use self::exec_session::{ShareExecInput, ShareExecSession};
pub use self::exec_types::{
    ExecCommand, ExecId, ExecJobView, ExecLifecycleState, ExecProviderStatus, ExecStart,
    ExecTerminal, ExecTerminalKind,
};
pub use self::fs::{ShareExportConfig, SharedRoot};
pub use self::identity::{DirectCodeRotation, IdentityRepair, IdentityRepairAction, ShareIdentity};
pub(crate) use self::identity_store::with_matching_identity_generation;
pub(crate) use self::legacy_direct_actions::mark_legacy_answer_attempt;
pub use self::legacy_direct_actions::{
    decide_legacy_direct_request, delete_legacy_direct_request, reconcile_legacy_identity,
    refresh_legacy_request_expiry, retry_legacy_direct_answer, revoke_legacy_direct_request,
};
pub use self::legacy_direct_request::{
    LegacyDirectAnswer, LegacyDirectDecisionDelivery, LegacyDirectDecisionSource,
    LegacyDirectDecisionState, LegacyDirectDeliveryState, LegacyDirectPresenceEvidence,
    LegacyDirectRequestEntry, MAX_LEGACY_DIRECT_REQUESTS, MAX_LEGACY_PRESENCE_FUTURE_SECS,
};
pub use self::profile_persistence::ProfileChange;
pub(crate) use self::profiles::ProfileRevision;
pub use self::profiles::ShareProfiles;
pub use self::service::ShareService;
pub use self::types::{
    DirectAccessState, DirectContact, DirectGrant, DirectGrantState, ExecGrantTarget, ExecRequest,
    ExecResult, PeerOpenTarget, PeerPresence, RoomMember, RoomProfile, ShareCmd, ShareEvent,
    ShareStatus,
};

pub fn core_now_secs() -> i64 {
    self::core::now_secs()
}

pub(crate) fn exec_provider_status() -> ExecProviderStatus {
    exec_platform::provider_status()
}

/// Runs the exact hidden supervisor invocation before CLI or GUI parsing.
/// Returning `Some` means the process was an internal supervisor and must exit.
pub fn run_exec_supervisor_if_requested(
    arguments: &[std::ffi::OsString],
) -> Option<std::io::Result<()>> {
    exec_platform::run_supervisor_if_requested(arguments)
}

#[cfg(debug_assertions)]
pub fn run_exec_platform_self_test() -> std::io::Result<()> {
    exec_platform::run_platform_self_test()
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
#[path = "core/direct_ledger_validation_tests.rs"]
mod direct_ledger_validation_tests;
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
