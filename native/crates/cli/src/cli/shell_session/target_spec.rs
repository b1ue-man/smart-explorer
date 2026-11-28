//! Where a persistent session runs: a saved SFTP remote over its SSH login,
//! a Share peer's Exec stream, or this device (the MCP `shell` tool, plan
//! docs/plaene/2026-11-28-ersetzen-masken-shell, M6). `elevated` runs the
//! session's shell with administrator or root rights: through `sudo` on an
//! SSH remote (`exec_ssh_sudo.rs`), asked by the peer's se on a Share peer,
//! through `engine::elevate` here; the whole session asks once.
use crate::creds::SavedConnection;
use crate::share::PeerOpenTarget;

use super::types::Dialect;

/// Where a session runs.
pub(in crate::cli) enum TargetSpec {
    Ssh {
        connection: SavedConnection,
        label: String,
        cwd: Option<String>,
        elevated: bool,
    },
    Share {
        peer: PeerOpenTarget,
        label: String,
        /// The Share folder the peer starts the shell in (`ExecStart::share_cwd`).
        share_cwd: Option<String>,
        elevated: bool,
    },
    /// This device: `start` runs the session's shell, which reads its
    /// commands from standard input in `dialect`.
    Local {
        label: String,
        start: crate::elevate::ElevatedSpec,
        dialect: Dialect,
        elevated: bool,
    },
}

impl TargetSpec {
    /// The selector as given (`@label`, endpoint, `local`).
    pub(in crate::cli) fn label(&self) -> &str {
        match self {
            Self::Ssh { label, .. } | Self::Share { label, .. } | Self::Local { label, .. } => {
                label
            }
        }
    }

    /// Identity of the remote: equal for every selector of the same saved
    /// connection or peer; an elevated session is another one.
    pub(in crate::cli) fn key(&self) -> String {
        let (base, elevated) = match self {
            Self::Ssh {
                connection,
                elevated,
                ..
            } => (format!("ssh {}", connection.account()), *elevated),
            Self::Share { peer, elevated, .. } => {
                (format!("share {}", peer.endpoint_prefix()), *elevated)
            }
            Self::Local { elevated, .. } => ("local".to_string(), *elevated),
        };
        if elevated {
            format!("{base} elevated")
        } else {
            base
        }
    }

    /// Working directory named by the selector (`@label:/dir`).
    pub(super) fn cwd(&self) -> Option<&str> {
        match self {
            Self::Ssh { cwd, .. } => cwd.as_deref(),
            Self::Share { .. } | Self::Local { .. } => None,
        }
    }

    /// The same target, elevated.
    pub(in crate::cli) fn elevated(mut self) -> Self {
        match &mut self {
            Self::Ssh { elevated, .. }
            | Self::Share { elevated, .. }
            | Self::Local { elevated, .. } => *elevated = true,
        }
        self
    }

    /// A Share target that starts in the Share folder `folder`.
    pub(in crate::cli) fn in_share_folder(mut self, folder: Option<String>) -> Self {
        if let Self::Share { share_cwd, .. } = &mut self {
            *share_cwd = folder;
        }
        self
    }
}

/// A saved SFTP remote (`@label[:/dir]`, its `sftp://` endpoint) or a Share
/// peer (name, id, `share://` endpoint; omitted when exactly one is ready).
pub(in crate::cli) fn resolve_target(selector: Option<&str>) -> Result<TargetSpec, String> {
    let selector = selector.map(str::trim).filter(|value| !value.is_empty());
    if let Some(given) = selector {
        if let Some(target) = crate::cli::exec_ssh::ssh_target(given)? {
            return Ok(TargetSpec::Ssh {
                connection: target.connection,
                label: given.to_string(),
                cwd: target.cwd,
                elevated: false,
            });
        }
    }
    let peer = crate::cli::exec::select_target(selector)?;
    let label = selector
        .map(str::to_string)
        .unwrap_or_else(|| peer.endpoint_prefix());
    Ok(TargetSpec::Share {
        peer,
        label,
        share_cwd: None,
        elevated: false,
    })
}
