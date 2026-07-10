use crate::creds::{Protocol, SavedConnection};
use crate::vfs::BackendHandle;

pub(crate) struct Target {
    pub(crate) backend: BackendHandle,
    pub(crate) path: String,
    /// Stable identity of the filesystem namespace behind this target. CLI
    /// source/destination arguments are resolved independently, so Arc identity
    /// alone cannot tell that two handles address the same saved remote.
    namespace_key: String,
    /// A narrower identity for handles that can safely perform an in-backend
    /// rename. Saved entries with different roots may use different credentials,
    /// so they share a namespace key but deliberately not this key.
    rename_key: String,
    /// True when the resolved path is the root of a filesystem namespace or a
    /// saved connection's configured root. Destructive commands must require
    /// an explicit preserve-root override for these targets.
    preserved_root: bool,
    #[allow(dead_code)]
    pub(crate) net: Option<crate::net::NetConnection>,
}

impl Target {
    pub(crate) fn same_namespace(&self, other: &Self) -> bool {
        self.namespace_key == other.namespace_key
    }

    pub(crate) fn can_rename_with(&self, other: &Self) -> bool {
        self.rename_key == other.rename_key
    }

    pub(crate) fn is_preserved_root(&self) -> bool {
        self.preserved_root
    }

    #[cfg(test)]
    pub(crate) fn with_backend_key(
        backend: BackendHandle,
        path: String,
        backend_key: impl Into<String>,
    ) -> Self {
        let backend_key = backend_key.into();
        Self::with_backend_keys(backend, path, backend_key.clone(), backend_key)
    }

    #[cfg(test)]
    pub(crate) fn with_backend_key_preserved_root(
        backend: BackendHandle,
        path: String,
        backend_key: impl Into<String>,
    ) -> Self {
        let mut target = Self::with_backend_key(backend, path, backend_key);
        target.preserved_root = true;
        target
    }

    #[cfg(test)]
    pub(crate) fn with_backend_keys(
        backend: BackendHandle,
        path: String,
        namespace_key: impl Into<String>,
        rename_key: impl Into<String>,
    ) -> Self {
        Self {
            backend,
            path,
            namespace_key: namespace_key.into(),
            rename_key: rename_key.into(),
            preserved_root: false,
            net: None,
        }
    }
}

pub(crate) fn resolve(spec: &str) -> Result<Target, String> {
    if let Some((conn, path)) = saved_shorthand(spec)? {
        let preserved_root = same_normalized_path(&path, &conn.root);
        let (backend, opened, net) = open_connection_at(&conn, &path)?;
        return Ok(Target {
            backend,
            path: opened,
            namespace_key: saved_namespace_key(&conn),
            rename_key: saved_rename_key(&conn),
            preserved_root,
            net,
        });
    }
    let normalized = normalize_local(spec);
    let (namespace_key, rename_key) = endpoint_backend_keys(&normalized);
    let saved_configured_root =
        crate::connect::saved_and_path(&normalized).map(|(connection, _)| connection.root);
    let (backend, path) = crate::connect::resolve_endpoint(&normalized)?;
    let preserved_root = is_endpoint_namespace_root(&normalized, &path)
        || saved_configured_root
            .as_deref()
            .is_some_and(|root| same_normalized_path(&path, root));
    Ok(Target {
        backend,
        path,
        namespace_key,
        rename_key,
        preserved_root,
        net: None,
    })
}

fn saved_namespace_key(c: &SavedConnection) -> String {
    if c.protocol == Protocol::Share {
        // UNC targets are all in the host filesystem namespace. Keeping this
        // aligned with raw local/UNC paths also catches aliasing between a saved
        // share and its direct UNC spelling.
        "local".to_string()
    } else {
        // The configured root is a starting path, not a separate filesystem.
        // Excluding it makes two saved entries for the same remote account
        // compare as one namespace, which is required for alias-safe checks.
        format!(
            "remote:{}://{}@{}:{}",
            c.protocol.as_str(),
            c.user,
            c.host.to_lowercase(),
            c.port
        )
    }
}

fn saved_rename_key(c: &SavedConnection) -> String {
    if c.protocol == Protocol::Share {
        "local".to_string()
    } else {
        format!("saved:{}", c.account())
    }
}

fn endpoint_backend_keys(spec: &str) -> (String, String) {
    if let Some((peer, path)) = crate::share::PeerOpenTarget::from_endpoint(spec) {
        let namespace_key = format!("peer:{}", peer.endpoint_prefix());
        let scope = peer_rename_scope(&path).unwrap_or_else(|| format!("virtual:{path}"));
        let rename_key = format!("{namespace_key}:mount:{scope}");
        return (namespace_key, rename_key);
    }
    if spec.starts_with("gdrive://") {
        return ("gdrive".to_string(), "gdrive".to_string());
    }
    if let Some((saved, _)) = crate::connect::saved_and_path(spec) {
        return (saved_namespace_key(&saved), saved_rename_key(&saved));
    }
    ("local".to_string(), "local".to_string())
}

fn peer_rename_scope(path: &str) -> Option<String> {
    let mut parts = path
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty());
    let first = parts.next()?;
    if first == "Verbindungen" {
        return parts.next().map(|second| format!("{first}/{second}"));
    }
    Some(first.to_string())
}

fn open_connection_at(
    c: &SavedConnection,
    path: &str,
) -> Result<(BackendHandle, String, Option<crate::net::NetConnection>), String> {
    if c.protocol == Protocol::Share {
        let secret = crate::creds::get_secret(&c.account());
        let net = crate::net::NetConnection::connect(
            &c.root,
            non_empty(&c.user).as_deref(),
            secret.as_deref(),
        )
        .map_err(|e| e.to_string())?;
        // `saved_shorthand` already resolved the suffix below `c.root`.
        // Joining the root again would produce `//server/share/server/share`.
        let path = normalize_local(path);
        return Ok((
            std::sync::Arc::new(crate::vfs::LocalBackend::new(&path)),
            path,
            Some(net),
        ));
    }
    let (backend, opened) = crate::connect::open_saved_at(c, path)?;
    Ok((backend, opened, None))
}

fn saved_shorthand(spec: &str) -> Result<Option<(SavedConnection, String)>, String> {
    let Some(rest) = spec.strip_prefix('@') else {
        return Ok(None);
    };
    let conns = crate::creds::load_connections();
    saved_shorthand_from(rest, spec, conns)
}

fn saved_shorthand_from(
    rest: &str,
    spec: &str,
    conns: Vec<SavedConnection>,
) -> Result<Option<(SavedConnection, String)>, String> {
    let mut matches: Vec<(SavedConnection, String, usize)> = Vec::new();
    for c in conns {
        for key in [c.display(), c.account()] {
            let prefix = format!("{key}:");
            if rest.starts_with(&prefix) {
                let suffix = &rest[prefix.len()..];
                matches.push((c.clone(), join_under(&c.root, suffix)?, prefix.len()));
            }
        }
    }
    matches.sort_by_key(|(_, _, len)| std::cmp::Reverse(*len));
    let Some((_, _, longest)) = matches.first().cloned() else {
        return Err(format!("unknown saved connection shorthand: {spec}"));
    };
    matches.retain(|(_, _, len)| *len == longest);
    matches.dedup_by(|a, b| a.0.account() == b.0.account());
    if matches.len() > 1 {
        let candidates = matches
            .iter()
            .map(|(c, _, _)| c.account())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!("saved connection label is ambiguous: {candidates}"));
    }
    Ok(matches.pop().map(|(c, p, _)| (c, p)))
}

fn join_under(root: &str, suffix: &str) -> Result<String, String> {
    let root = norm_root(root);
    let suffix = suffix.trim().replace('\\', "/");
    let suffix = suffix.trim_start_matches('/');
    if suffix
        .split('/')
        .any(|component| matches!(component, "." | ".."))
    {
        return Err("saved connection shorthand cannot contain '.' or '..' components".into());
    }
    if suffix.as_bytes().contains(&0) {
        return Err("saved connection shorthand cannot contain a NUL byte".into());
    }
    if suffix.is_empty() {
        Ok(root)
    } else {
        Ok(format!("{}/{}", root.trim_end_matches('/'), suffix))
    }
}

fn same_normalized_path(left: &str, right: &str) -> bool {
    norm_root(left) == norm_root(right)
}

fn is_endpoint_namespace_root(spec: &str, path: &str) -> bool {
    if crate::share::PeerOpenTarget::from_endpoint(spec).is_some() {
        return is_slash_root(path);
    }
    if spec.starts_with("gdrive://") {
        return path.is_empty() || is_slash_root(path);
    }
    is_local_namespace_root(path)
}

fn is_local_namespace_root(path: &str) -> bool {
    let path = normalize_local(path);
    if is_slash_root(&path) {
        return true;
    }
    let trimmed = path.trim_end_matches('/');
    if is_exact_drive_root(trimmed) {
        return true;
    }
    if let Some(rest) = trimmed.strip_prefix("//") {
        let components = rest.split('/').filter(|part| !part.is_empty()).count();
        return components <= 2;
    }
    false
}

fn is_slash_root(path: &str) -> bool {
    path.starts_with('/') && path.chars().all(|ch| ch == '/')
}

fn is_exact_drive_root(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn norm_root(root: &str) -> String {
    let root = root.trim().replace('\\', "/");
    if root.is_empty() {
        "/".to_string()
    } else if root.starts_with('/') || is_drive_root(&root) {
        let root = root.trim_end_matches('/');
        if root.is_empty() {
            "/".to_string()
        } else {
            root.to_string()
        }
    } else {
        format!("/{root}")
    }
}

fn is_drive_root(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

fn normalize_local(spec: &str) -> String {
    spec.replace('\\', "/")
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

#[cfg(test)]
mod tests {
    use crate::creds::{AuthKind, Protocol, SavedConnection};

    use super::{
        endpoint_backend_keys, is_local_namespace_root, join_under, peer_rename_scope,
        saved_namespace_key, saved_rename_key, saved_shorthand_from,
    };

    #[test]
    fn shorthand_suffix_is_under_configured_root() {
        assert_eq!(
            join_under("/home/alice", "/docs/a.txt").unwrap(),
            "/home/alice/docs/a.txt"
        );
        assert_eq!(join_under("/", "/docs/a.txt").unwrap(), "/docs/a.txt");
        assert_eq!(join_under("/", "").unwrap(), "/");
        assert_eq!(join_under("base", "leaf").unwrap(), "/base/leaf");
        assert!(join_under("/home/alice", "../escape").is_err());
        assert!(join_under("/home/alice", "docs/./file").is_err());
    }

    #[test]
    fn namespace_roots_are_identified_without_matching_children() {
        for root in [
            "/",
            "////",
            "C:",
            "C:/",
            "//server/share",
            "\\\\server\\share\\",
        ] {
            assert!(is_local_namespace_root(root), "expected root: {root}");
        }
        for child in ["/tmp", "C:/tmp", "//server/share/dir"] {
            assert!(!is_local_namespace_root(child), "expected child: {child}");
        }
    }

    #[test]
    fn duplicate_saved_labels_fail_with_candidates() {
        let a = conn("prod", "alice", "/srv/a");
        let b = conn("prod", "bob", "/srv/b");
        let err = saved_shorthand_from("prod:/logs", "@prod:/logs", vec![a.clone(), b.clone()])
            .unwrap_err();
        assert!(err.contains("ambiguous"));
        assert!(err.contains(&a.account()));
        assert!(err.contains(&b.account()));
    }

    #[test]
    fn account_shorthand_can_disambiguate_duplicate_labels() {
        let a = conn("prod", "alice", "/srv/a");
        let b = conn("prod", "bob", "/srv/b");
        let spec = format!("{}:/logs", b.account());
        let (found, path) = saved_shorthand_from(&spec, &format!("@{spec}"), vec![a, b.clone()])
            .unwrap()
            .unwrap();
        assert_eq!(found.account(), b.account());
        assert_eq!(path, "/srv/b/logs");
    }

    #[test]
    fn endpoint_backend_keys_separate_peers_but_join_each_peer_paths() {
        let docs_a = endpoint_backend_keys("share://direct/contact-a/Docs/a.txt");
        let docs_b = endpoint_backend_keys("share://direct/contact-a/Docs/b.txt");
        let other = endpoint_backend_keys("share://direct/contact-a/Other/a.txt");
        assert_eq!(docs_a.0, docs_b.0);
        assert_eq!(docs_a.1, docs_b.1);
        assert_eq!(docs_a.0, other.0);
        assert_ne!(docs_a.1, other.1);
        assert_ne!(
            endpoint_backend_keys("share://direct/contact-a/Docs"),
            endpoint_backend_keys("share://direct/contact-b/Docs")
        );
        assert_eq!(
            endpoint_backend_keys("C:/tmp/a"),
            ("local".to_string(), "local".to_string())
        );
        assert_eq!(
            endpoint_backend_keys("gdrive:///a"),
            ("gdrive".to_string(), "gdrive".to_string())
        );
        assert_eq!(
            peer_rename_scope("/Verbindungen/Production/a.txt").as_deref(),
            Some("Verbindungen/Production")
        );
        assert_eq!(peer_rename_scope("/Verbindungen"), None);
    }

    #[test]
    fn saved_remote_roots_share_one_backend_namespace() {
        let first = conn("first", "alice", "/srv/one");
        let second = conn("second", "alice", "/srv/two");
        assert_eq!(saved_namespace_key(&first), saved_namespace_key(&second));
        assert_ne!(saved_rename_key(&first), saved_rename_key(&second));
    }

    fn conn(label: &str, user: &str, root: &str) -> SavedConnection {
        SavedConnection {
            protocol: Protocol::Sftp,
            host: "example.test".into(),
            port: 22,
            user: user.into(),
            auth: AuthKind::Password,
            root: root.into(),
            label: label.into(),
            use_agent: false,
        }
    }
}
