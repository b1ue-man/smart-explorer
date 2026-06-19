use crate::creds::{AuthKind, Protocol, SavedConnection};
use crate::vfs::BackendHandle;
use std::sync::Arc;

/// A live remote (SFTP/FTP) session held by the app while browsing it.
pub struct RemoteState {
    pub backend: BackendHandle,
    pub label: String,
    /// `Some(version)` when an SSH remote agent is active for this session (#24);
    /// drives the "Agent" status indicator. `None` = plain backend.
    pub agent_version: Option<String>,
    /// For an opened ZIP archive: the local folder to return to when the archive
    /// is closed. `None` for real network connections.
    pub zip_return: Option<String>,
    /// The concrete SFTP backend behind this session, kept so the SSH remote
    /// agent can be activated LATER on an already-established connection
    /// (#24, runtime opt-in). `None` for non-SFTP.
    pub sftp: Option<Arc<crate::sftp::SftpBackend>>,
    /// Saved-connection account key (if this came from a saved connection), so a
    /// later agent activation can persist the choice. `None` for ad-hoc/non-SFTP.
    pub account: Option<String>,
    /// `proto://user@host:port` for this session (`None` for local/share/zip), so
    /// favourites and per-folder settings can be keyed by connection + path
    /// (a re-openable endpoint URL) rather than a bare path.
    pub endpoint_prefix: Option<String>,
}

/// Editable Connect-dialog state.
#[derive(Clone)]
pub struct ConnectForm {
    pub protocol: Protocol,
    pub host: String,
    pub port: String,
    pub user: String,
    pub password: String,
    pub use_key: bool,
    pub keyfile: String,
    pub passphrase: String,
    pub root: String,
    pub unc: String, // network share path
    pub save: bool,
    pub label: String,
    /// Opt-in SSH remote agent (#24); SFTP only.
    pub use_agent: bool,
}

impl Default for ConnectForm {
    fn default() -> Self {
        ConnectForm {
            protocol: Protocol::Sftp,
            host: String::new(),
            port: "22".into(),
            user: String::new(),
            password: String::new(),
            use_key: false,
            keyfile: String::new(),
            passphrase: String::new(),
            root: "/".into(),
            unc: String::new(),
            save: false,
            label: String::new(),
            use_agent: false,
        }
    }
}

impl ConnectForm {
    /// Pre-fill the form from a saved connection (the secret is loaded
    /// separately from the keyring at connect time).
    pub fn from_saved(c: &SavedConnection) -> Self {
        let (use_key, keyfile) = match &c.auth {
            AuthKind::Key { path } => (true, path.clone()),
            AuthKind::Password => (false, String::new()),
        };
        ConnectForm {
            protocol: c.protocol,
            host: c.host.clone(),
            port: c.port.to_string(),
            user: c.user.clone(),
            password: String::new(),
            use_key,
            keyfile,
            passphrase: String::new(),
            root: if c.protocol.is_url() {
                c.root.clone()
            } else {
                "/".into()
            },
            unc: if c.protocol.is_url() {
                String::new()
            } else {
                c.root.clone()
            },
            save: true,
            label: c.label.clone(),
            use_agent: c.use_agent,
        }
    }
}

/// Outcome of a connect attempt.
pub enum ConnectResult {
    Ok(Connected),
    Err(String),
}

pub struct Connected {
    /// Some for SFTP/FTP (walked via rscan); None for a share.
    pub remote: Option<RemoteState>,
    /// Some for an authenticated share (kept alive while browsing).
    pub net: Option<crate::net::NetConnection>,
    /// Navigation target: the remote root path or the UNC path.
    pub target: String,
    pub label: String,
}
