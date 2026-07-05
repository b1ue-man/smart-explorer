use crate::creds::{Protocol, SavedConnection};
use crate::vfs::BackendHandle;

pub(crate) struct Target {
    pub(crate) backend: BackendHandle,
    pub(crate) path: String,
    #[allow(dead_code)]
    pub(crate) net: Option<crate::net::NetConnection>,
}

pub(crate) fn resolve(spec: &str) -> Result<Target, String> {
    if let Some((conn, path)) = saved_shorthand(spec)? {
        let (backend, opened, net) = open_connection_at(&conn, &path)?;
        return Ok(Target {
            backend,
            path: opened,
            net,
        });
    }
    let (backend, path) = crate::connect::resolve_endpoint(&normalize_local(spec))?;
    Ok(Target {
        backend,
        path,
        net: None,
    })
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
        let path = join_under(&c.root.replace('\\', "/"), path);
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
                matches.push((c.clone(), join_under(&c.root, suffix), prefix.len()));
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

fn join_under(root: &str, suffix: &str) -> String {
    let root = norm_root(root);
    let suffix = suffix.trim().replace('\\', "/");
    let suffix = suffix.trim_start_matches('/');
    if suffix.is_empty() {
        root
    } else {
        format!("{}/{}", root.trim_end_matches('/'), suffix)
    }
}

fn norm_root(root: &str) -> String {
    let root = root.trim().replace('\\', "/");
    if root.is_empty() {
        "/".to_string()
    } else if root.starts_with('/') || is_drive_root(&root) || root.starts_with("//") {
        root.trim_end_matches('/').to_string()
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

    use super::{join_under, saved_shorthand_from};

    #[test]
    fn shorthand_suffix_is_under_configured_root() {
        assert_eq!(
            join_under("/home/alice", "/docs/a.txt"),
            "/home/alice/docs/a.txt"
        );
        assert_eq!(join_under("/", "/docs/a.txt"), "/docs/a.txt");
        assert_eq!(join_under("base", "leaf"), "/base/leaf");
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
