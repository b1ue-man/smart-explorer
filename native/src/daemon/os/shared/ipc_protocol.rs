use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::net::TcpStream;
use std::time::Duration;

use super::line::{read_line_limited_from_stream, MAX_IPC_LINE};

const MAX_EVENT_BYTES: usize = 384 * 1024;
const MAX_LEGACY_REQUEST_BYTES: usize = 256 * 1024;
const MAX_CANDIDATE_BYTES: usize = 128 * 1024;
const MAX_STATUS_TEXT_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum MountBackendScheme {
    Local,
    Sftp,
    Ftp,
    Webdav,
    GoogleDrive,
    Peer,
}

impl From<crate::vfs::Scheme> for MountBackendScheme {
    fn from(value: crate::vfs::Scheme) -> Self {
        match value {
            crate::vfs::Scheme::Local => Self::Local,
            crate::vfs::Scheme::Sftp => Self::Sftp,
            crate::vfs::Scheme::Ftp => Self::Ftp,
            crate::vfs::Scheme::Webdav => Self::Webdav,
            crate::vfs::Scheme::GDrive => Self::GoogleDrive,
            crate::vfs::Scheme::Peer => Self::Peer,
        }
    }
}

impl From<MountBackendScheme> for crate::vfs::Scheme {
    fn from(value: MountBackendScheme) -> Self {
        match value {
            MountBackendScheme::Local => Self::Local,
            MountBackendScheme::Sftp => Self::Sftp,
            MountBackendScheme::Ftp => Self::Ftp,
            MountBackendScheme::Webdav => Self::Webdav,
            MountBackendScheme::GoogleDrive => Self::GDrive,
            MountBackendScheme::Peer => Self::Peer,
        }
    }
}

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountHostConfig {
    pub id: crate::mount::MountId,
    pub drive: crate::mount::DriveSelection,
    pub mode: crate::mount::MountMode,
    #[serde(default)]
    pub metadata: crate::mount::MountMetadataPolicy,
    #[serde(default)]
    pub cache: crate::mount::MountCachePolicy,
    #[serde(default)]
    pub runtime_preference: crate::mount::MountRuntimePreference,
    pub label: String,
}

impl MountHostConfig {
    pub fn with_cache_policy(mut self, cache: crate::mount::MountCachePolicy) -> Self {
        self.cache = cache;
        self
    }

    pub fn with_runtime_preference(
        mut self,
        preference: crate::mount::MountRuntimePreference,
    ) -> Self {
        self.runtime_preference = preference;
        self
    }
}

impl From<&crate::mount::MountConfig> for MountHostConfig {
    fn from(config: &crate::mount::MountConfig) -> Self {
        Self {
            id: config.id.clone(),
            drive: config.drive,
            mode: config.mode,
            metadata: config.metadata,
            cache: config.cache,
            runtime_preference: config.runtime_preference,
            label: config.label.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ShareWorkerSnapshot {
    pub events: Vec<crate::share::ShareEvent>,
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
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub(super) enum IpcRequest {
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
    },
    DrainShareEvents {
        token: String,
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
            Self::Ping { token }
            | Self::RefreshShare { token }
            | Self::ShareCommand { token, .. }
            | Self::MutateExecGrant { token, .. }
            | Self::DrainShareEvents { token }
            | Self::OpenShare { token, .. }
            | Self::ProbeShareMount { token, .. }
            | Self::ExecShare { token, .. }
            | Self::ExecStream { token, .. }
            | Self::ExecJobs { token }
            | Self::CancelExec { token, .. }
            | Self::StartMount { token, .. }
            | Self::StopMount { token, .. }
            | Self::ListMounts { token }
            | Self::RetryMount { token, .. } => Some(token),
            Self::MountHostAttach { .. }
            | Self::MountHostBackend { .. }
            | Self::MountHostStatus { .. } => None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub(super) enum IpcResponse {
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
    ExecResult {
        result: crate::share::ExecResult,
    },
    ExecGrantMutation {
        result: super::ipc_host::exec_grant_journal::ExecGrantPersistResult,
    },
    ExecReady {
        exec_id: crate::share::ExecId,
    },
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
    Err {
        msg: String,
    },
}

pub(super) fn write_response(stream: &mut TcpStream, response: &IpcResponse) -> io::Result<()> {
    let mut line = serde_json::to_string(response).map_err(eio)?;
    line.push('\n');
    if line.len() > MAX_IPC_LINE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ipc response exceeds line budget",
        ));
    }
    stream.write_all(line.as_bytes())?;
    stream.flush()
}

pub(super) fn bound_snapshot_for_ipc(mut snapshot: ShareWorkerSnapshot) -> ShareWorkerSnapshot {
    let dropped_events = retain_newest_with_budget(&mut snapshot.events, MAX_EVENT_BYTES);
    let dropped_legacy = retain_newest_with_budget(
        &mut snapshot.pending_direct_requests,
        MAX_LEGACY_REQUEST_BYTES,
    );
    let dropped_candidates =
        retain_newest_with_budget(&mut snapshot.candidates, MAX_CANDIDATE_BYTES);
    if let Some(error) = &mut snapshot.last_error {
        truncate_utf8(error, MAX_STATUS_TEXT_BYTES);
    }
    truncate_utf8(&mut snapshot.relay_url, MAX_STATUS_TEXT_BYTES);

    if dropped_events + dropped_legacy + dropped_candidates > 0 {
        snapshot.events.push(crate::share::ShareEvent::Error(format!(
            "Share status truncated transient backlog: events={dropped_events}, legacy_requests={dropped_legacy}, candidates={dropped_candidates}; durable request/profile state is complete"
        )));
    }

    while encoded_response_len(&snapshot) > MAX_IPC_LINE {
        if !snapshot.events.is_empty() {
            snapshot.events.remove(0);
            continue;
        }
        if snapshot.candidates.pop().is_some() {
            continue;
        }
        if snapshot.pending_direct_requests.pop().is_some() {
            continue;
        }
        break;
    }
    snapshot
}

fn retain_newest_with_budget<T: Serialize>(values: &mut Vec<T>, budget: usize) -> usize {
    let original_len = values.len();
    let mut used = 2usize;
    let mut kept = Vec::new();
    for value in std::mem::take(values).into_iter().rev() {
        let bytes = serde_json::to_vec(&value)
            .map(|encoded| encoded.len().saturating_add(1))
            .unwrap_or(budget.saturating_add(1));
        if used.saturating_add(bytes) <= budget {
            used += bytes;
            kept.push(value);
        }
    }
    kept.reverse();
    *values = kept;
    original_len.saturating_sub(values.len())
}

fn encoded_response_len(snapshot: &ShareWorkerSnapshot) -> usize {
    serde_json::to_vec(&IpcResponse::ShareEvents {
        snapshot: Box::new(snapshot.clone()),
    })
    .map(|encoded| encoded.len().saturating_add(1))
    .unwrap_or(usize::MAX)
}

fn truncate_utf8(value: &mut String, max: usize) {
    if value.len() <= max {
        return;
    }
    let mut end = max;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
}

pub(super) fn set_stream_timeout(stream: &TcpStream, timeout: Option<Duration>) {
    let _ = stream.set_read_timeout(timeout);
    let _ = stream.set_write_timeout(timeout);
}

pub(super) fn write_request(stream: &mut TcpStream, req: &IpcRequest) -> io::Result<()> {
    let mut line = serde_json::to_string(req).map_err(eio)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    stream.flush()
}

pub(super) fn read_response(stream: &mut TcpStream) -> io::Result<IpcResponse> {
    let mut line = String::new();
    read_line_limited_from_stream(stream, &mut line, MAX_IPC_LINE)?;
    serde_json::from_str(line.trim()).map_err(eio)
}

fn eio<E: std::fmt::Display>(error: E) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
#[path = "ipc_protocol_tests.rs"]
mod tests;
