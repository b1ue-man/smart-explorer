//! Target strings of the MCP tools: exactly the `se` grammar (`@label:/path`,
//! saved `sftp://`/`ftp://`/`ftps://`/`webdav://` endpoints, `share://…`,
//! `gdrive:///…`, `vault://…`, local and UNC paths), with the session rules:
//! a working root (`root` tool) makes relative targets paths below it, and
//! without one local paths must be absolute, because the agent host decides
//! the server's working directory. Connections stay open for the session
//! like the explorer's tabs (`tab_cache.rs`): one login per saved
//! SFTP/FTP/FTPS/WebDAV connection, one connection per Share mount, and the
//! working root's connection (SMB, Windows shares, UNC, Google Drive,
//! vaults) for every path below it. A connection that failed is dropped and
//! opened again by the next call.
use super::root_path::{self, Place, WorkRoot};
use super::tab_cache::{self, Failure, Slots};
use crate::cli::target::{self, Target};
use crate::creds::{Protocol, SavedConnection};
use crate::share::OperationOrigin;
use crate::vfs::BackendHandle;
use std::cell::RefCell;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(super) struct Targets {
    logins: Slots<BackendHandle>,
    ssh: Slots<Arc<crate::sftp::SftpBackend>>,
    /// Resolved anchors (a Share mount, the working root) whose connection
    /// the paths below them reuse.
    tabs: Slots<Arc<Target>>,
    root: Mutex<Option<Arc<WorkRoot>>>,
}

/// Where the connection of a resolved target is kept.
enum Kept {
    /// Opened for this call only.
    Fresh,
    /// A kept login; `probe` is the saved connection's root.
    Login { probe: String },
    /// Below a kept anchor, which also keeps its connection lease alive.
    Tab(Arc<Target>),
}

/// A resolved target of one call.
pub(super) struct Resolved {
    target: Target,
    kept: Kept,
    /// Removing it needs `allow_root`: a filesystem or connection root, or
    /// the working root itself.
    protected: bool,
    /// The absolute spelling of the target.
    pub(super) spec: String,
}

impl std::ops::Deref for Resolved {
    type Target = Target;

    fn deref(&self) -> &Target {
        &self.target
    }
}

impl Resolved {
    pub(super) fn protected(&self) -> bool {
        self.protected
    }
}

impl Targets {
    /// A target of a file tool: a person (the agent's user) waits on it,
    /// so a Share device's transfers yield to it (E4, 2026-11-14).
    pub(super) fn resolve(&self, spec: &str) -> Result<Resolved, String> {
        self.resolve_for(spec, OperationOrigin::User)
    }

    /// A target for operations of `origin` (`Job` for `copy` and `move`).
    pub(super) fn resolve_for(
        &self,
        spec: &str,
        origin: OperationOrigin,
    ) -> Result<Resolved, String> {
        match root_path::place(spec, self.root())? {
            Place::Under(root, rel) => self.resolve_under(&root, &rel, origin),
            Place::Absolute(spec) => {
                require_absolute_local(&spec)?;
                self.resolve_absolute(&spec, origin)
            }
        }
    }

    /// The working root, if one is set.
    pub(super) fn root(&self) -> Option<Arc<WorkRoot>> {
        self.root.lock().ok().and_then(|root| root.clone())
    }

    /// Makes `spec` (absolute, or relative to the current root) the working
    /// root once it is a reachable folder; returns it with its resolution.
    pub(super) fn set_root(&self, spec: &str) -> Result<(Arc<WorkRoot>, Resolved), String> {
        let absolute = match root_path::place(spec, self.root())? {
            Place::Under(root, rel) => root_path::join_spec(&root.spec, &rel),
            Place::Absolute(spec) => {
                require_absolute_local(&spec)?;
                spec
            }
        };
        let kind = root_path::kind_of(&absolute)?;
        let provisional = WorkRoot {
            spec: absolute.clone(),
            kind,
            path: String::new(),
        };
        let resolved = self.resolve_under(&provisional, "", OperationOrigin::User)?;
        let meta = resolved
            .backend
            .stat(&resolved.path)
            .map_err(|error| self.io_error(&resolved, error))?;
        if !meta.is_dir || meta.is_symlink {
            return Err(format!(
                "{absolute} is not a folder (a link counts only at the folder it points to); a working root must be a reachable folder"
            ));
        }
        let root = Arc::new(WorkRoot {
            spec: absolute,
            kind,
            path: resolved.path.clone(),
        });
        *self
            .root
            .lock()
            .map_err(|_| "working root is poisoned".to_string())? = Some(root.clone());
        Ok((root, resolved))
    }

    /// Clears the working root; returns the one that was set. Its
    /// connections stay open for absolute targets of the session.
    pub(super) fn clear_root(&self) -> Option<Arc<WorkRoot>> {
        self.root.lock().ok().and_then(|mut root| root.take())
    }

    fn resolve_under(
        &self,
        root: &WorkRoot,
        rel: &str,
        origin: OperationOrigin,
    ) -> Result<Resolved, String> {
        let spec = root_path::join_spec(&root.spec, rel);
        if !root.kind.shares_root_connection() {
            let mut resolved = self.resolve_absolute(&spec, origin)?;
            resolved.protected |= rel.is_empty();
            return Ok(resolved);
        }
        let base = self.tab(&root.spec, origin)?;
        let path = if rel.is_empty() {
            base.path.clone()
        } else {
            crate::cli::ops::join(&base.path, rel)
        };
        let target = base.at_path(path);
        let protected = rel.is_empty()
            || target.is_preserved_root()
            || root_path::namespace_root(root.kind, &target.path)
            || saved_root(&spec);
        Ok(Resolved {
            target,
            kept: Kept::Tab(base),
            protected,
            spec,
        })
    }

    fn resolve_absolute(&self, spec: &str, origin: OperationOrigin) -> Result<Resolved, String> {
        let normalized = spec.replace('\\', "/");
        if let Some((peer, path)) = crate::share::PeerOpenTarget::from_endpoint(&normalized) {
            // One connection per Share mount, the scope of renames there.
            if let Some(scope) = root_path::peer_scope(&path) {
                let anchor = format!("{}{scope}", peer.endpoint_prefix());
                let base = self.tab(&anchor, origin)?;
                return Ok(Resolved {
                    target: base.at_path(path),
                    kept: Kept::Tab(base),
                    protected: false,
                    spec: spec.to_string(),
                });
            }
        }
        let login = RefCell::new(None::<String>);
        let open =
            |connection: &SavedConnection, path: &str| -> Result<(BackendHandle, String), String> {
                let (backend, opened, probe) = self.open(connection, path)?;
                if let Some(probe) = probe {
                    *login.borrow_mut() = Some(probe);
                }
                Ok((backend, opened))
            };
        let target = target::resolve_with(spec, Some(&open), origin)?;
        let kept = match login.into_inner() {
            Some(probe) => Kept::Login { probe },
            None => Kept::Fresh,
        };
        Ok(Resolved {
            protected: target.is_preserved_root(),
            target,
            kept,
            spec: spec.to_string(),
        })
    }

    /// The kept resolution of `anchor`, opened by the first call.
    fn tab(&self, anchor: &str, origin: OperationOrigin) -> Result<Arc<Target>, String> {
        let key = format!("{origin:?}|{anchor}");
        let (base, _) = self.tabs.get_or_open(&key, || {
            let open = |connection: &SavedConnection, path: &str| {
                self.open(connection, path)
                    .map(|(backend, opened, _)| (backend, opened))
            };
            target::resolve_with(anchor, Some(&open), origin).map(Arc::new)
        })?;
        Ok(base)
    }

    /// The SSH login of a saved SFTP connection for remote commands, reused
    /// for the session (each command still gets its own exec channel).
    pub(super) fn ssh(
        &self,
        connection: &SavedConnection,
    ) -> Result<Arc<crate::sftp::SftpBackend>, String> {
        self.ssh
            .get_or_open(&connection.account(), || {
                crate::connect::open_saved_ssh(connection)
            })
            .map(|(backend, _)| backend)
    }

    /// Drops the SSH login of `connection` after a command could not start
    /// on it; the next command logs in again.
    pub(super) fn ssh_failed(&self, connection: &SavedConnection) {
        self.ssh.evict(&connection.account());
    }

    /// Text of a failed operation on `target`; drops its kept connection
    /// when the failure is the connection's (a probe decides unclear kinds).
    pub(super) fn io_error(&self, target: &Resolved, error: std::io::Error) -> String {
        match tab_cache::classify(error.kind()) {
            Failure::Connection => self.evict(target),
            Failure::Entry => {}
            Failure::Unknown => self.after_failure(target),
        }
        error.to_string()
    }

    /// After a failure known only by its text: drops the kept connection of
    /// `target` if it no longer reaches the path it was opened at.
    pub(super) fn after_failure(&self, target: &Resolved) {
        let probe = match &target.kept {
            Kept::Fresh => return,
            Kept::Login { probe } => probe.clone(),
            Kept::Tab(base) => base.path.clone(),
        };
        if tab_cache::connection_broken(&*target.backend, &probe) {
            self.evict(target);
        }
    }

    fn evict(&self, target: &Resolved) {
        if matches!(target.kept, Kept::Fresh) {
            return;
        }
        let backend = &target.backend;
        self.tabs
            .evict_where(|base| tab_cache::same_backend(&base.backend, backend));
        self.logins
            .evict_where(|login| tab_cache::same_backend(login, backend));
    }

    /// Opens a saved connection at `path`; a kept login also names the
    /// path that probes it (the connection's configured root).
    fn open(
        &self,
        connection: &SavedConnection,
        path: &str,
    ) -> Result<(BackendHandle, String, Option<String>), String> {
        // SMB maps paths onto its share inside the connector; it is opened as
        // the terminal command does (and kept as the anchor of a root).
        if !matches!(
            connection.protocol,
            Protocol::Sftp | Protocol::Ftp | Protocol::Ftps | Protocol::Webdav
        ) {
            let (backend, opened) = crate::connect::open_saved_at(connection, path)?;
            return Ok((backend, opened, None));
        }
        let mut opened = None;
        let (backend, fresh) = self.logins.get_or_open(&connection.account(), || {
            let (backend, path) = crate::connect::open_saved_at(connection, path)?;
            opened = Some(path);
            Ok(backend)
        })?;
        let path = match opened {
            Some(opened) if fresh => opened,
            _ => connector_path(path),
        };
        Ok((backend, path, Some(connector_path(&connection.root))))
    }
}

/// Whether `spec` names the configured root of a saved connection.
fn saved_root(spec: &str) -> bool {
    let saved = if spec.starts_with('@') {
        target::saved_shorthand(spec).ok().flatten()
    } else {
        crate::connect::saved_and_path(&spec.replace('\\', "/"))
    };
    saved.is_some_and(|(connection, path)| {
        let norm = |text: &str| text.replace('\\', "/").trim_end_matches('/').to_string();
        norm(&path) == norm(&connection.root)
    })
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
            "empty target; set a working root with `root`, or use an absolute local path, @label:/path, or an endpoint".into(),
        );
    }
    let remote = spec.starts_with('@') || spec.contains("://");
    if remote || root_path::is_absolute_local(spec) {
        return Ok(());
    }
    Err(format!(
        "local path {spec:?} is relative and no working root is set; the server's working directory is chosen by the agent host, so give an absolute path or set a root with `root`"
    ))
}

#[cfg(test)]
mod tests {
    use super::{connector_path, require_absolute_local, Targets};
    use crate::cli::mcp::root_path::RootKind;

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

    /// A local working root: relative targets below it, `..` bounded, the
    /// root itself protected from `remove`, absolute targets unchanged.
    #[test]
    fn mcp_root_task_targets_resolve_below_a_local_working_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        let file = dir.path().join("src").join("main.rs");
        std::fs::write(&file, "fn main() {}\n").unwrap();
        let targets = Targets::default();
        // Without a root a relative target is refused as before.
        let refused = targets.resolve("src/main.rs").err().unwrap();
        assert!(refused.contains("absolute"), "{refused}");
        // A file is no working root.
        assert!(targets.set_root(&file.to_string_lossy()).is_err());
        let root = dir.path().to_string_lossy().into_owned();
        let (work, _) = targets.set_root(&root).unwrap();
        assert_eq!(work.kind, RootKind::Local);
        let main = targets.resolve("src/main.rs").unwrap();
        assert_eq!(main.backend.stat(&main.path).unwrap().size, 13);
        assert!(!main.protected());
        let same = targets.resolve("./src/../src/main.rs").unwrap();
        assert_eq!(same.path, main.path);
        assert_eq!(
            targets.resolve(&file.to_string_lossy()).unwrap().path,
            main.path
        );
        for spelled in ["", ".", "  "] {
            assert!(targets.resolve(spelled).unwrap().protected(), "{spelled:?}");
        }
        let above = targets.resolve("..").err().unwrap();
        assert!(above.contains("leaves the working root"), "{above}");
        // An absolute target elsewhere stays valid.
        let other = tempfile::tempdir().unwrap();
        let elsewhere = targets.resolve(&other.path().to_string_lossy()).unwrap();
        assert!(elsewhere.backend.stat(&elsewhere.path).unwrap().is_dir);
        // A relative root is taken below the current one.
        let (sub, _) = targets.set_root("src").unwrap();
        assert!(sub.spec.ends_with("src"), "{}", sub.spec);
        assert_eq!(targets.resolve("main.rs").unwrap().path, main.path);
        assert!(targets.set_root("../..").is_err());
        assert!(targets.clear_root().is_some());
        assert!(targets.root().is_none());
        assert!(targets.resolve("main.rs").is_err());
    }
}
