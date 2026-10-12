//! Target strings of the MCP tools: exactly the `se` grammar (`@label:/path`,
//! saved `sftp://`/`ftp://`/`ftps://`/`webdav://` endpoints, `share://…`,
//! `gdrive:///…`, local and UNC paths), with two session rules: local paths
//! must be absolute, because the agent host decides the server's working
//! directory; and one login per saved SFTP/FTP/FTPS/WebDAV connection is
//! reused for the whole session instead of a new handshake per call.
use crate::cli::target::{self, Target};
use crate::creds::{Protocol, SavedConnection};
use crate::vfs::BackendHandle;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(super) struct Targets {
    logins: Mutex<HashMap<String, BackendHandle>>,
    ssh: Mutex<HashMap<String, Arc<crate::sftp::SftpBackend>>>,
}

impl Targets {
    pub(super) fn resolve(&self, spec: &str) -> Result<Target, String> {
        let spec = spec.trim();
        require_absolute_local(spec)?;
        let open = |connection: &SavedConnection, path: &str| self.open(connection, path);
        target::resolve_with(spec, Some(&open))
    }

    /// The SSH login of a saved SFTP connection for remote commands, reused
    /// for the session (each command still gets its own exec channel).
    pub(super) fn ssh(
        &self,
        connection: &SavedConnection,
    ) -> Result<Arc<crate::sftp::SftpBackend>, String> {
        let key = connection.account();
        if let Some(backend) = self
            .ssh
            .lock()
            .map_err(|_| "connection cache is poisoned".to_string())?
            .get(&key)
            .cloned()
        {
            return Ok(backend);
        }
        let backend = crate::connect::open_saved_ssh(connection)?;
        Ok(self
            .ssh
            .lock()
            .map_err(|_| "connection cache is poisoned".to_string())?
            .entry(key)
            .or_insert(backend)
            .clone())
    }

    fn open(
        &self,
        connection: &SavedConnection,
        path: &str,
    ) -> Result<(BackendHandle, String), String> {
        // SMB maps paths onto its share inside the connector; it is opened as
        // the terminal command does.
        if !matches!(
            connection.protocol,
            Protocol::Sftp | Protocol::Ftp | Protocol::Ftps | Protocol::Webdav
        ) {
            return crate::connect::open_saved_at(connection, path);
        }
        let key = connection.account();
        if let Some(backend) = self
            .logins
            .lock()
            .map_err(|_| "connection cache is poisoned".to_string())?
            .get(&key)
            .cloned()
        {
            return Ok((backend, connector_path(path)));
        }
        let (backend, opened) = crate::connect::open_saved_at(connection, path)?;
        self.logins
            .lock()
            .map_err(|_| "connection cache is poisoned".to_string())?
            .entry(key)
            .or_insert_with(|| backend.clone());
        Ok((backend, opened))
    }
}

/// The path the connector reports for a freshly opened saved connection.
fn connector_path(path: &str) -> String {
    if path.is_empty() {
        "/".to_string()
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

fn require_absolute_local(spec: &str) -> Result<(), String> {
    if spec.is_empty() {
        return Err(
            "empty target; use an absolute local path, @label:/path, or an endpoint".into(),
        );
    }
    let remote = spec.starts_with('@') || spec.contains("://");
    if remote || is_absolute_local(spec) {
        return Ok(());
    }
    Err(format!(
        "local path {spec:?} is relative; the server's working directory is chosen by the agent host, so give an absolute path"
    ))
}

fn is_absolute_local(spec: &str) -> bool {
    let unc = spec.starts_with("\\\\") || spec.starts_with("//");
    unc || std::path::Path::new(spec).is_absolute()
}

#[cfg(test)]
mod tests {
    use super::{connector_path, require_absolute_local};

    #[test]
    fn mcp_task_local_targets_must_be_absolute() {
        assert!(require_absolute_local("@prod:/srv").is_ok());
        assert!(require_absolute_local("share://direct/a/Docs").is_ok());
        assert!(require_absolute_local("gdrive:///Docs").is_ok());
        assert!(require_absolute_local("//server/share/x").is_ok());
        assert!(require_absolute_local("relative/file.txt")
            .unwrap_err()
            .contains("absolute"));
        assert!(require_absolute_local("").is_err());
        #[cfg(windows)]
        assert!(require_absolute_local("C:\\Users\\a").is_ok());
        #[cfg(not(windows))]
        assert!(require_absolute_local("/tmp/a").is_ok());
        assert_eq!(connector_path(""), "/");
        assert_eq!(connector_path("srv"), "/srv");
        assert_eq!(connector_path("/srv/x"), "/srv/x");
    }
}
