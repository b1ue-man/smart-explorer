use serde::{Deserialize, Serialize};

use super::line::MAX_IPC_LINE;

#[path = "ipc_protocol_bounds.rs"]
mod bounds;
pub(super) use self::bounds::bound_snapshot_for_ipc;
#[cfg(test)]
use self::bounds::encoded_response_len;

// Drive-host wire types (plan 2026-10-17 B2, R1); re-exported unchanged.
#[path = "ipc_protocol_mount.rs"]
mod mount;
pub(super) use self::mount::MountBackendScheme;
pub use self::mount::MountHostConfig;
#[path = "ipc_protocol_access.rs"]
mod access;
pub(crate) use self::access::{AccessIpcReply, AccessIpcRequest, AccountChoice, ExecNoticeView};
#[path = "ipc_protocol_vault.rs"]
mod vault;
pub(crate) use self::vault::{
    CompareHead, CompareItemWire, CompareSideWire, ResolveFailure, SealPurposeWire, SecretText,
    VaultEventView, VaultIpcReply, VaultIpcRequest, VaultSiteView, VaultStatusView,
};
#[path = "ipc_protocol_apps.rs"]
pub(super) mod apps;
#[path = "ipc_protocol_login.rs"]
pub(crate) mod login;
#[path = "ipc_protocol_screen.rs"]
pub(super) mod screen;
#[path = "ipc_protocol_tcp.rs"]
pub(super) mod tcp;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum MountDeleteDisposition {
    Recycle,
    Permanent,
    Unsupported,
}

impl From<crate::vfs::DeleteDisposition> for MountDeleteDisposition {
    fn from(value: crate::vfs::DeleteDisposition) -> Self {
        match value {
            crate::vfs::DeleteDisposition::Recycle => Self::Recycle,
            crate::vfs::DeleteDisposition::Permanent => Self::Permanent,
            crate::vfs::DeleteDisposition::Unsupported => Self::Unsupported,
        }
    }
}

impl From<MountDeleteDisposition> for crate::vfs::DeleteDisposition {
    fn from(value: MountDeleteDisposition) -> Self {
        match value {
            MountDeleteDisposition::Recycle => Self::Recycle,
            MountDeleteDisposition::Permanent => Self::Permanent,
            MountDeleteDisposition::Unsupported => Self::Unsupported,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(super) struct MountBackendCapabilities {
    pub(super) rename_overwrites: bool,
    pub(super) staged_write_create: bool,
    pub(super) staged_write_replace: bool,
    pub(super) staged_namespace_replace: bool,
    #[serde(default)]
    pub(super) case_sensitive_paths: bool,
    pub(super) delete_disposition: MountDeleteDisposition,
    pub(super) parallelism: u8,
}

impl MountBackendCapabilities {
    pub(super) fn from_backend(backend: &crate::vfs::BackendHandle) -> Self {
        let staged_write = backend.staged_write_capabilities("/");
        Self {
            rename_overwrites: staged_write.namespace_replace,
            staged_write_create: staged_write.create,
            staged_write_replace: staged_write.replace,
            staged_namespace_replace: staged_write.namespace_replace,
            case_sensitive_paths: backend.case_sensitive_paths("/"),
            delete_disposition: backend.delete_disposition().into(),
            // The daemon proxy itself admits at most eight active requests.
            parallelism: backend.parallelism().clamp(1, 8) as u8,
        }
    }

    pub(super) fn staged_write(self) -> crate::vfs::StagedWriteCapabilities {
        crate::vfs::StagedWriteCapabilities {
            create: self.staged_write_create,
            replace: self.staged_write_replace,
            namespace_replace: self.staged_namespace_replace,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(super) struct MountPathCapabilitiesWire {
    pub(super) staged_write_create: bool,
    pub(super) staged_write_replace: bool,
    pub(super) staged_namespace_replace: bool,
    pub(super) root_confined: bool,
}

impl From<crate::vfs::MountPathCapabilities> for MountPathCapabilitiesWire {
    fn from(value: crate::vfs::MountPathCapabilities) -> Self {
        Self {
            staged_write_create: value.staged_write.create,
            staged_write_replace: value.staged_write.replace,
            staged_namespace_replace: value.staged_write.namespace_replace,
            root_confined: value.root_confinement.is_enforced(),
        }
    }
}

impl From<MountPathCapabilitiesWire> for crate::vfs::MountPathCapabilities {
    fn from(value: MountPathCapabilitiesWire) -> Self {
        Self {
            staged_write: crate::vfs::StagedWriteCapabilities {
                create: value.staged_write_create,
                replace: value.staged_write_replace,
                namespace_replace: value.staged_namespace_replace,
            },
            root_confinement: if value.root_confined {
                crate::vfs::RootConfinement::Enforced
            } else {
                crate::vfs::RootConfinement::Unverified
            },
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ShareWorkerSnapshot {
    pub events: Vec<crate::share::ShareEvent>,
    /// Additive route facts; absent when reading a worker predating this API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routes: Option<crate::share::ShareRouteSnapshot>,
    /// Canonical daemon-owned profile state. Consumers must replace stale
    /// cached copies instead of replaying worker events as profile writes.
    #[serde(default)]
    pub profiles: crate::share::ShareProfiles,
    #[serde(default)]
    pub(crate) profile_revision: crate::share::ProfileRevision,
    #[serde(default)]
    pub(crate) exec_grant_retry:
        Option<super::ipc_host::exec_grant_journal::ExecGrantPersistResult>,
    #[serde(default)]
    pub pending_direct_requests: Vec<crate::share::PeerPresence>,
    pub running: bool,
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub last_error: Option<String>,
    pub relay_url: String,
    pub candidates: Vec<String>,
    /// Local-network presence and uplink-sharing state.
    #[serde(default)]
    pub lan: crate::share::LanStatus,
    /// This device's own unexpired discovery offers. Unlike `events`, this is
    /// the daemon's persistent view and is never consumed by a reader.
    #[serde(default)]
    pub discovery_offers: Vec<crate::share::OwnDiscoveryOffer>,
}

/// Result data of a Share command that the caller needs, returned over IPC
/// instead of being discarded.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShareCommandReply {
    Applied,
    DiscoveryOffer {
        offer: crate::share::OwnDiscoveryOffer,
    },
    /// The offer was accepted but ended before the daemon could report it.
    DiscoveryOfferEnded {
        offer_id: String,
        reason: crate::share::DiscoveryOfferStopReason,
    },
    DiscoveryExchange {
        exchange_id: String,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub(super) enum IpcRequest {
    DiscoveryPairing {
        token: String,
        request: super::discovery_pairing_protocol::DiscoveryPairingRequest,
    },
    RuntimeCall {
        token: String,
        hello: crate::mcp_launcher::RuntimeHello,
        daemon_generation: String,
        request: Box<IpcRequest>,
    },
    RuntimeHello {
        token: String,
        hello: crate::mcp_launcher::RuntimeHello,
    },
    Ping {
        token: String,
    },
    RefreshShare {
        token: String,
    },
    ShareCommand {
        token: String,
        cmd: crate::share::ShareCmd,
    },
    MutateExecGrant {
        token: String,
        target: crate::share::ExecGrantTarget,
        enabled: bool,
        /// Run-as account and program list; absent keeps the grant's own
        /// (clients that predate run-as send none).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settings: Option<crate::share::ExecGrantSettings>,
    },
    DrainShareEvents {
        token: String,
    },
    /// Same snapshot as `DrainShareEvents` without consuming UI events.
    ShareSnapshot {
        token: String,
    },
    /// Same answer and authentication as `ShareSnapshot`, for the read-only
    /// view of se Desktop (`share_worker_peek`). `ShareSnapshot`, which every
    /// caller of `share_worker_snapshot` sends, lifts the restriction of a
    /// worker started for the drive restore; this variant does not
    /// (`run_start_reason.rs`). A daemon that predates it refuses it.
    ShareSnapshotPeek {
        token: String,
    },
    AnalyzeShare {
        token: String,
        target: crate::share::PeerOpenTarget,
        root: String,
        #[serde(default)]
        node_budget: Option<u64>,
    },
    OpenShare {
        token: String,
        target: crate::share::PeerOpenTarget,
    },
    ProbeShareMount {
        token: String,
        target: crate::share::PeerOpenTarget,
        root: String,
    },
    ExecShare {
        token: String,
        target: crate::share::PeerOpenTarget,
        req: crate::share::ExecRequest,
    },
    ExecStream {
        token: String,
        target: crate::share::PeerOpenTarget,
        start: crate::share::ExecStart,
    },
    /// Requires the independent owner/cancellation channel before input handoff.
    ExecStreamControlled {
        token: String,
        target: crate::share::PeerOpenTarget,
        start: crate::share::ExecStart,
    },
    ExecControl {
        token: String,
        exec_id: crate::share::ExecId,
        control_token: String,
    },
    ExecJobs {
        token: String,
    },
    CancelExec {
        token: String,
        target: super::exec_state::ExecCancelTarget,
    },
    StartMount {
        token: String,
        config: crate::mount::MountConfig,
    },
    StopMount {
        token: String,
        id: crate::mount::MountId,
    },
    ListMounts {
        token: String,
    },
    RetryMount {
        token: String,
        id: crate::mount::MountId,
    },
    Access {
        token: String,
        request: AccessIpcRequest,
    },
    Drive {
        token: String,
        req: super::drive_modules::ipc_protocol_drives::DriveIpcRequest,
    },
    Vault {
        token: String,
        req: VaultIpcRequest,
    },
    Screen {
        token: String,
        req: screen::ScreenIpcRequest,
    },
    Tcp {
        token: String,
        req: tcp::TcpIpcRequest,
    },
    Apps {
        token: String,
        req: apps::AppsIpcRequest,
    },
    Login {
        token: String,
        req: login::LoginIpcRequest,
    },
    /// A drive host's request beyond attach, backend and status (R5); the
    /// session token of its mount authorizes it, never the daemon token.
    MountHostExt {
        id: crate::mount::MountId,
        session_token: String,
        req: super::drive_modules::ipc_protocol_mount_host::MountHostRequest,
    },
    MountHostAttach {
        id: crate::mount::MountId,
        launch_token: String,
    },
    MountHostBackend {
        id: crate::mount::MountId,
        backend_token: String,
    },
    MountHostStatus {
        id: crate::mount::MountId,
        session_token: String,
        status: crate::mount::MountStatus,
        /// Present only at host-owned recovery boundaries. Older/simpler
        /// status reports deliberately leave the daemon's conservative state
        /// unchanged.
        #[serde(default)]
        recovery: Option<crate::mount::MountRecovery>,
        /// Compatibility signal for the immediately preceding daemon/host.
        #[serde(default)]
        recovery_required: Option<bool>,
    },
}

impl IpcRequest {
    pub(super) fn daemon_token(&self) -> Option<&str> {
        match self {
            // `SpoolKey` carries the session token of its drive host: only
            // that host may fetch the key (`ipc_host_ext.rs`), so the daemon
            // token of the GUI or the CLI must not reach it.
            Self::Vault {
                req: VaultIpcRequest::SpoolKey { .. },
                ..
            } => None,
            Self::RuntimeCall { token, .. }
            | Self::DiscoveryPairing { token, .. }
            | Self::RuntimeHello { token, .. }
            | Self::Ping { token }
            | Self::RefreshShare { token }
            | Self::ShareCommand { token, .. }
            | Self::MutateExecGrant { token, .. }
            | Self::DrainShareEvents { token }
            | Self::ShareSnapshot { token }
            | Self::ShareSnapshotPeek { token }
            | Self::OpenShare { token, .. }
            | Self::AnalyzeShare { token, .. }
            | Self::ProbeShareMount { token, .. }
            | Self::ExecShare { token, .. }
            | Self::ExecStream { token, .. }
            | Self::ExecStreamControlled { token, .. }
            | Self::ExecControl { token, .. }
            | Self::ExecJobs { token }
            | Self::CancelExec { token, .. }
            | Self::StartMount { token, .. }
            | Self::StopMount { token, .. }
            | Self::ListMounts { token }
            | Self::RetryMount { token, .. }
            | Self::Access { token, .. }
            | Self::Drive { token, .. }
            | Self::Vault { token, .. }
            | Self::Screen { token, .. }
            | Self::Tcp { token, .. }
            | Self::Apps { token, .. }
            | Self::Login { token, .. } => Some(token),
            Self::MountHostAttach { .. }
            | Self::MountHostBackend { .. }
            | Self::MountHostStatus { .. }
            | Self::MountHostExt { .. } => None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub(super) enum IpcResponse {
    DiscoveryPairing {
        reply: super::discovery_pairing_protocol::DiscoveryPairingReply,
    },
    MountRecovery {
        recovery: crate::mount::InactiveMountRecovery,
    },
    MountRecoveries {
        recoveries: Vec<crate::mount::InactiveMountRecovery>,
    },
    RuntimeHello {
        reply: crate::mcp_launcher::RuntimeHelloReply,
    },
    Pong {
        #[serde(default)]
        version: String,
        #[serde(default)]
        generation: String,
        #[serde(default)]
        initialized: bool,
    },
    Ok,
    RefreshOk {
        running: bool,
    },
    OpenOk {
        label: String,
        status: crate::share::ShareStatus,
    },
    MountPathCapabilities {
        capabilities: MountPathCapabilitiesWire,
    },
    ShareEvents {
        snapshot: Box<ShareWorkerSnapshot>,
    },
    ShareCommand {
        reply: ShareCommandReply,
    },
    ExecResult {
        result: crate::share::ExecResult,
    },
    ExecGrantMutation {
        result: super::ipc_host::exec_grant_journal::ExecGrantPersistResult,
    },
    ExecReady {
        exec_id: crate::share::ExecId,
    },
    ExecControlledReady {
        exec_id: crate::share::ExecId,
        control_token: String,
    },
    ExecControlReady,
    ExecJobs {
        snapshot: super::exec_state::ExecJobsSnapshot,
    },
    ExecCancelled {
        found: bool,
    },
    Mount {
        mount: crate::mount::MountSnapshot,
    },
    Mounts {
        mounts: Vec<crate::mount::MountSnapshot>,
    },
    MountHostReady {
        config: MountHostConfig,
        scheme: MountBackendScheme,
        capabilities: MountBackendCapabilities,
        session_token: String,
        backend_token: String,
    },
    MountHostStop,
    Access(AccessIpcReply),
    Vault(VaultIpcReply),
    Screen(screen::ScreenIpcReply),
    Tcp(tcp::TcpIpcReply),
    Apps(apps::AppsIpcReply),
    Login(login::LoginIpcReply),
    MountHostExt(super::drive_modules::ipc_protocol_mount_host::MountHostReply),
    Devices {
        status: super::drive_modules::devices_ipc::DevicesStatus,
    },
    Err {
        msg: String,
        /// Structured code of the failure (C3.2, C7.5); older readers ignore the field.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
}

#[path = "ipc_protocol_io.rs"]
mod io_wire;
pub(super) use io_wire::{read_response, set_stream_timeout, write_request, write_response};

#[cfg(test)]
#[path = "ipc_protocol_tests.rs"]
mod tests;
