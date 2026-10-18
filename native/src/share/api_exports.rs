// Share API and test registrations, included at the feature root.
pub use lan_permission::{lan_firewall_repair_available, request_lan_firewall_repair};
pub use lan_permission_job::{poll_lan_firewall_repair, start_lan_firewall_repair};

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
pub use self::direct_reciprocal::{
    DirectReciprocalApply, DirectReciprocalConflict, DirectReciprocalError, DirectReciprocalPeer,
    DirectRelationMaterial,
};
pub use self::direct_reciprocal_persistence::{
    persist_reciprocal_direct_peer, DirectReciprocalPersistenceError,
    DirectReciprocalPersistenceOutcome,
};
pub use self::direct_relation_actions::{
    allow_direct_peer_again, set_direct_share_back, RelationChange,
};
pub use self::direct_request_tombstone::DirectRequestTombstone;
pub use self::direct_signal_event::DirectSignalEvent;
pub use self::discovery_offer_book::{DiscoveryOfferBook, OfferLookup, OwnDiscoveryOffer};
pub use self::discovery_pin::{
    discovery_pin_strength, suggest_discovery_pin, DiscoveryPinStrength,
    DISCOVERY_MAX_FAILED_PAIRINGS, DISCOVERY_MAX_OFFER_SECS, DISCOVERY_MIN_PIN_CHARS,
};
pub use self::discovery_relation_store::DiscoveryRelationOutcome;
pub(crate) use self::discovery_signal_state::MAX_DISCOVERY_ALIAS_BYTES;
pub use self::discovery_signal_types::{
    DiscoveryAdvertisement, DiscoveryCommand, DiscoveryEvent, DiscoveryExchangeHandle,
    DiscoveryKind, DiscoveryOfferHandle, DiscoveryOfferStopReason, DiscoveryPin,
    DiscoveryPublishTarget, PairingCloseReason, PairingPacketKind, DISCOVERY_PAIRING_SUITE,
    DISCOVERY_PAIRING_VERSION, DISCOVERY_PIN_MAX_BYTES,
};
pub(crate) use self::exec_client::{ExecClientEvent, ExecClientInput};
pub use self::exec_grant_runtime::ExecGrantMutation;
pub use self::exec_policy::{ExecGrant, ExecGrantSettings, ExecRunAs};
pub(crate) use self::exec_session::{ShareExecInput, ShareExecSession};
pub use self::exec_targets::{
    exec_target_views, resolve_exec_target, ExecTargetRelation, ExecTargetView,
};
// Name of the capability for the terminal sessions built on top of Exec.
#[allow(unused_imports)]
pub(crate) use self::exec_terminal::EXEC_TERMINAL_FEATURE;
pub use self::exec_terminal::{ExecTerminalRequest, ExecTerminalSize};
pub use self::exec_types::{
    ExecCommand, ExecId, ExecJobView, ExecLifecycleState, ExecProviderStatus, ExecStart,
    ExecTerminal, ExecTerminalKind,
};
pub use self::export_config::{
    ExportAccess, ShareExportConfig, SharedConnection, SharedRoot, DIRECT_EXPORT_SCOPE,
};
pub use self::identity::{DirectCodeRotation, IdentityRepair, IdentityRepairAction, ShareIdentity};
pub(crate) use self::identity_store::with_matching_identity_generation;
pub use self::lan_presence::{LanAnnouncement, LanEvent, LanPresence};
pub use self::lan_presence_match::{LanSighting, LAN_PRESENCE_TTL_SECS};
pub use self::lan_privacy::LanProof;
pub use self::lan_settings::LanSettings;
pub use self::lan_status::{
    LanFacility, LanPeerView, LanStatus, LinkView, UplinkSharingState, UplinkView,
};
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
#[cfg(target_os = "android")]
pub(crate) use self::platform_exec::{exec_host_activity, set_exec_host_listener};
pub use self::profile_migration::AutoHomeMigration;
pub use self::profile_persistence::ProfileChange;
pub use self::profile_policy_actions::set_direct_peer_write;
pub(crate) use self::profiles::ProfileRevision;
pub use self::profiles::ShareProfiles;
pub(crate) use self::relation_rights::profiles_differ_beyond_runtime;
pub use self::removed_direct_peers::{
    ForgottenDirectPeer, PairingOrigin, RemovedDirectPeer, MAX_REMOVED_DIRECT_PEERS,
};
pub use self::service::ShareService;
pub(crate) use self::transport_options::migrate_server_file;
pub use self::types::{
    DirectAccessState, DirectContact, DirectGrant, DirectGrantState, DirectRelationFlags,
    DirectRequestPolicy, ExecGrantTarget, ExecRequest, ExecResult, MemberUpsert, PeerOpenTarget,
    PeerPresence, PresenceApply, RelationRuntime, RoomMember, RoomMemberAdmission, RoomMemberFlags,
    RoomPolicy, RoomProfile, ShareCmd, ShareCmdResult, ShareEvent, ShareStatus,
};

pub fn core_now_secs() -> i64 {
    self::core::now_secs()
}

pub(crate) fn exec_provider_status() -> ExecProviderStatus {
    exec_platform::provider_status()
}

/// The last `lines` entries of this host's Exec audit log, oldest first.
pub(crate) fn exec_audit_tail(lines: usize) -> std::io::Result<Vec<String>> {
    exec_audit::tail(lines)
}

pub(crate) use self::exec_audit::{ExecAuditEntry, MAX_VIEW_ENTRIES as MAX_EXEC_AUDIT_VIEW};

/// The last `lines` entries (at most `MAX_EXEC_AUDIT_VIEW`) of this host's
/// Exec audit log as the GUIs show them: newest first, every field cleaned.
pub(crate) fn exec_audit_entries(lines: usize) -> std::io::Result<Vec<ExecAuditEntry>> {
    exec_audit::view(lines)
}

/// Whether a grant may name another local account than the worker's own:
/// derived from the same host facts as `exec_account_choices` without asking
/// the account service (Linux with a root worker, or with the account
/// service set up for this worker's user).
pub(crate) fn exec_run_as_supported() -> bool {
    exec_platform::run_as_supported()
}

/// "Ausführen als" of this host: the state of its account service and every
/// account choice with availability, reason and next step (PRM-M6, PRM-M7,
/// D.2.4). Unlike `exec_run_as_supported` it may ask the account service
/// (Windows: one logon probe per listed account), so callers on a UI thread
/// run it in the background.
pub(crate) fn exec_account_choices() -> exec_helper_protocol::AccountChoices {
    #[cfg(any(target_os = "linux", windows))]
    {
        platform_exec::account_choices()
    }
    #[cfg(target_os = "android")]
    {
        exec_platform::own_account_choices(access_catalog::HelperStatus {
            state: access_catalog::HelperState::Unavailable,
            version: None,
            detail: Some("Android runs commands only as the app".into()),
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "android", windows)))]
    {
        exec_platform::own_account_choices(access_catalog::HelperStatus {
            state: access_catalog::HelperState::NotNeeded,
            version: None,
            detail: None,
        })
    }
}

/// Sessions of another account on this host have no network credentials
/// (no network drives, no EFS files) and no visible windows: true on
/// Windows, where the account service logs on without password (S4U) and
/// runs in session 0 (C2). The GUIs show the hint of C2 then.
pub(crate) fn exec_account_session_limits() -> bool {
    cfg!(windows)
}

// Setting up the account service as administrator (`se share exec accounts
// setup|allow|disallow|update|remove`, PRM-M6 on Linux, PRM-M7 on Windows):
// both platforms offer the same types. On Windows the elevations
// `PkexecGraphical` and `PkexecTerminal` ask through UAC.
#[cfg(target_os = "linux")]
#[allow(unused_imports)] // keeps the Linux re-export of the invoking user in use
pub(crate) use self::platform_exec::invoking_uid as exec_accounts_invoking_uid;
#[cfg(any(target_os = "linux", windows))]
pub(crate) use self::platform_exec::{
    exec_accounts_apply, Elevation as ExecAccountsElevation, ExecAccountsAction,
    ExecAccountsOutcome,
};

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
#[path = "core/copy_paste_task_fixture.rs"]
mod copy_paste_task_fixture;
#[cfg(test)]
pub(crate) use copy_paste_task_fixture::CopyPastePeerFixture;
#[cfg(test)]
#[path = "core/copy_paste_task_tests.rs"]
mod copy_paste_task_tests;
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
#[path = "core/remote_drive_task_stop_tests.rs"]
mod remote_drive_task_stop_tests;
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
