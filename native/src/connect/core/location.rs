//! Application locators keep their historical literal backend paths. In
//! particular, `%20` names are not decoded into spaces during a sync reopen.
use crate::creds::{Protocol, SavedConnection};

use super::endpoint::parse_remote_url;

/// Why a general sync refuses a vault locator as one of its sides.
pub(crate) const VAULT_SIDE_REFUSED: &str = "Tresororte (vault://, vault-store://) sind keine Seite allgemeiner Sync-Jobs: Tresore gleicht der Tresor-Sync ab";

pub(crate) enum EndpointSpec {
    Local(String),
    Saved(String),
    Drive(String),
    Peer(crate::share::PeerOpenTarget, String),
    /// `vault://<SITE>/<path>`: the cleartext view of an entered vault site
    /// (`<SITE>` as written: ID, ID prefix or label; the path literal).
    Vault {
        site: String,
        path: String,
    },
    /// `vault-store://<site-id>`: the encrypted storage of a vault site,
    /// resolved without a wrapper to the site's own location.
    VaultStore {
        site: String,
    },
}

impl EndpointSpec {
    pub(crate) fn parse(endpoint: &str) -> Result<Self, String> {
        if endpoint.trim().is_empty() || endpoint.contains('\0') {
            return Err("Leerer oder ungültiger Pfad".into());
        }
        if endpoint.starts_with('/')
            || endpoint.starts_with('\\')
            || (endpoint.as_bytes().get(1) == Some(&b':')
                && endpoint.as_bytes()[0].is_ascii_alphabetic())
        {
            return Ok(Self::Local(stored_local_root(endpoint)));
        }
        let Some((scheme, rest)) = endpoint.split_once("://") else {
            if let Some((scheme, _)) = endpoint.split_once(':') {
                if !scheme.is_empty()
                    && scheme
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                {
                    return Err(format!("Ungültige Remote-Adresse: {scheme} benötigt ://"));
                }
            }
            return Ok(Self::Local(stored_local_root(endpoint)));
        };
        // A colon within an absolute filesystem path is still a filename.
        if !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        {
            return Ok(Self::Local(stored_local_root(endpoint)));
        }
        let scheme = scheme.to_ascii_lowercase();
        match scheme.as_str() {
            "gdrive" => Ok(Self::Drive(format!("/{}", rest.trim_start_matches('/')))),
            "share" => crate::share::PeerOpenTarget::from_endpoint(&format!("share://{rest}"))
                .map(|(target, path)| Self::Peer(target, path))
                .ok_or_else(|| "Ungültige Share-Adresse".into()),
            "sftp" | "ftp" | "ftps" | "webdav" | "smb" => {
                if rest.is_empty() {
                    return Err("Ungültige Remote-Adresse".into());
                }
                Ok(Self::Saved(format!("{scheme}://{rest}")))
            }
            crate::vault::VAULT_SCHEME => {
                crate::vault::VaultLocator::parse(endpoint).map(|locator| Self::Vault {
                    site: locator.site,
                    path: locator.path,
                })
            }
            crate::vault::VAULT_STORE_SCHEME => crate::vault::VaultStoreLocator::parse(endpoint)
                .map(|locator| Self::VaultStore { site: locator.site }),
            _ => Err(format!("Nicht unterstütztes Pfadprotokoll: {scheme}")),
        }
    }
}

fn stored_local_root(path: &str) -> String {
    // Persisted baselines include the exact root string. Preserve old Windows
    // separators while comparing normalized copies only for overlap checks.
    if path.len() == 2 && path.as_bytes()[0].is_ascii_alphabetic() && path.ends_with(':') {
        format!("{path}/")
    } else {
        path.to_string()
    }
}

pub(crate) fn local_root(path: &str) -> String {
    let bytes = path.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    let root = if drive || path.starts_with("\\\\") {
        path.replace('\\', "/")
    } else {
        path.to_string()
    };
    if drive && root.len() == 2 {
        format!("{root}/")
    } else {
        root
    }
}

/// Choose credentials within one authority by the most specific saved root.
/// Exact legacy prefixes also preserve usernames containing URL delimiters.
pub(crate) fn saved_location<'a>(
    connections: &'a [SavedConnection],
    endpoint: &str,
) -> Option<(&'a SavedConnection, String)> {
    let parsed = parse_remote_url(endpoint);
    connections
        .iter()
        .enumerate()
        .filter(|(_, connection)| connection.protocol.is_url())
        .filter_map(|(index, connection)| {
            let prefix = format!(
                "{}://{}@{}:{}",
                connection.protocol.as_str(),
                connection.user,
                connection.host,
                connection.port
            );
            let direct = endpoint
                .strip_prefix(&prefix)
                .filter(|path| path.is_empty() || path.starts_with('/'))
                .map(|path| {
                    if path.is_empty() {
                        "/".into()
                    } else {
                        path.to_string()
                    }
                });
            let path = direct.or_else(|| {
                let (protocol, user, host, port, path) = parsed.as_ref()?;
                (*protocol == connection.protocol
                    && *user == connection.user
                    && hosts_equal(host, &connection.host)
                    && *port == connection.port)
                    .then(|| path.clone())
            })?;
            let root = connection.root.trim_end_matches('/');
            let score = if paths_overlap_one_way(root, &path) {
                root.len() + 1
            } else {
                0
            };
            Some((connection, path, (score, std::cmp::Reverse(index))))
        })
        .max_by_key(|(_, _, score)| *score)
        .map(|(connection, path, _)| (connection, path))
}

fn hosts_equal(a: &str, b: &str) -> bool {
    a.trim_matches(['[', ']'])
        .eq_ignore_ascii_case(b.trim_matches(['[', ']']))
}

/// Pure endpoint validation shared by the editor, persistence and daemon.
/// Vault locators are no side of a sync job, mirror or other general sync:
/// a cleartext view would put vault cleartext into sync state and copies
/// outside the vault (C2.7, K6); vault sync jobs check their own sides.
pub(crate) fn validate_sync_endpoints(source: &str, target: &str) -> Result<(), String> {
    for side in [EndpointSpec::parse(source)?, EndpointSpec::parse(target)?] {
        if matches!(
            side,
            EndpointSpec::Vault { .. } | EndpointSpec::VaultStore { .. }
        ) {
            return Err(VAULT_SIDE_REFUSED.into());
        }
    }
    let source = endpoint_key(source);
    let target = endpoint_key(target);
    if source.0 == target.0 && paths_overlap(&source.1, &target.1) {
        return Err("Quelle und Ziel müssen verschieden und dürfen nicht ineinander verschachtelt sein (distinct, non-nested endpoints).".into());
    }
    Ok(())
}

fn endpoint_key(endpoint: &str) -> (String, String) {
    match EndpointSpec::parse(endpoint) {
        Ok(EndpointSpec::Local(path)) => {
            let path = local_root(&path);
            let windows = path.starts_with("//") || path.as_bytes().get(1) == Some(&b':');
            let path = if windows { path.to_lowercase() } else { path };
            ("local".into(), path_key(&path))
        }
        Ok(EndpointSpec::Drive(path)) => ("gdrive".into(), path_key(&path)),
        Ok(EndpointSpec::Vault { site, path }) => {
            (crate::vault::vault_location(&site, ""), path_key(&path))
        }
        Ok(EndpointSpec::VaultStore { site }) => {
            (crate::vault::vault_store_location(&site), String::new())
        }
        Ok(EndpointSpec::Peer(target, path)) => (target.endpoint_prefix(), path_key(&path)),
        Ok(EndpointSpec::Saved(url)) => {
            if let Some((proto, user, host, port, path)) = parse_remote_url(&url) {
                (
                    format!(
                        "{}://{}@{}:{}",
                        proto.as_str(),
                        user,
                        host.trim_matches(['[', ']']).to_ascii_lowercase(),
                        port
                    ),
                    path_key(&path),
                )
            } else {
                (url, String::new())
            }
        }
        Err(_) => (endpoint.into(), String::new()),
    }
}

pub(crate) fn paths_overlap(a: &str, b: &str) -> bool {
    let (a, b) = (path_key(a), path_key(b));
    paths_overlap_one_way(&a, &b) || paths_overlap_one_way(&b, &a)
}

fn paths_overlap_one_way(parent: &str, child: &str) -> bool {
    parent == child
        || child
            .strip_prefix(parent)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn path_key(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "." => {}
            ".." if parts
                .last()
                .is_some_and(|part| !part.is_empty() && *part != "..") =>
            {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/").trim_end_matches('/').to_string()
}

pub(super) fn parse_host_port(hostport: &str, protocol: Protocol) -> Option<(String, u16)> {
    let (host, port) = if let Some(rest) = hostport.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        host.parse::<std::net::Ipv6Addr>().ok()?;
        let port = if tail.is_empty() {
            protocol.default_port()
        } else {
            tail.strip_prefix(':')?.parse().ok()?
        };
        (host.to_string(), port)
    } else if hostport.matches(':').count() > 1 {
        // Accept historical unbracketed IPv6 + explicit port emitted by the picker.
        let (host, port) = hostport.rsplit_once(':')?;
        host.parse::<std::net::Ipv6Addr>().ok()?;
        (host.to_string(), port.parse().ok()?)
    } else if let Some((host, port)) = hostport.rsplit_once(':') {
        (host.to_string(), port.parse().ok()?)
    } else {
        (hostport.to_string(), protocol.default_port())
    };
    if host.is_empty() || host.contains([' ', '\\', '?', '#']) || port == 0 {
        None
    } else {
        Some((host, port))
    }
}

#[cfg(test)]
mod vault_locator_tests {
    use super::{validate_sync_endpoints, EndpointSpec, VAULT_SIDE_REFUSED};

    const SITE: &str = "0123456789abcdef0123456789abcdef";

    /// A compact, comparable form of a parse result.
    fn shape(endpoint: &str) -> String {
        match EndpointSpec::parse(endpoint) {
            Ok(EndpointSpec::Local(path)) => format!("local {path}"),
            Ok(EndpointSpec::Saved(url)) => format!("saved {url}"),
            Ok(EndpointSpec::Drive(path)) => format!("drive {path}"),
            Ok(EndpointSpec::Peer(target, path)) => {
                format!("peer {} {path}", target.endpoint_prefix())
            }
            Ok(EndpointSpec::Vault { site, path }) => format!("vault {site} {path}"),
            Ok(EndpointSpec::VaultStore { site }) => format!("vault-store {site}"),
            Err(error) => format!("error {error}"),
        }
    }

    #[test]
    fn vault_task_existing_locators_unchanged() {
        // Every stored form of v0.5.182 parses exactly as before.
        let table = [
            ("/home/anna/Docs", "local /home/anna/Docs"),
            ("C:", "local C:/"),
            (r"C:\Users\anna", r"local C:\Users\anna"),
            (r"\\server\share\dir", r"local \\server\share\dir"),
            ("relative/dir", "local relative/dir"),
            ("x:y", "local x:y"),
            ("a b://x", "local a b://x"),
            ("ab:cd", "error Ungültige Remote-Adresse: ab benötigt ://"),
            (
                "sftp://anna@nas:22/srv/%20a",
                "saved sftp://anna@nas:22/srv/%20a",
            ),
            ("SFTP://anna@nas/srv", "saved sftp://anna@nas/srv"),
            ("ftp://h/x", "saved ftp://h/x"),
            ("ftps://h:990/x", "saved ftps://h:990/x"),
            (
                "webdav://anna@cloud:443/dav",
                "saved webdav://anna@cloud:443/dav",
            ),
            ("smb://anna@nas/daten", "saved smb://anna@nas/daten"),
            ("sftp://", "error Ungültige Remote-Adresse"),
            ("gdrive:///Docs", "drive /Docs"),
            ("gdrive://Docs", "drive /Docs"),
            (
                "share://direct/contact-a/Gate/Sub",
                "peer share://direct/contact-a /Gate/Sub",
            ),
            (
                "http://example.org/x",
                "error Nicht unterstütztes Pfadprotokoll: http",
            ),
            (
                "vaults://x",
                "error Nicht unterstütztes Pfadprotokoll: vaults",
            ),
            ("", "error Leerer oder ungültiger Pfad"),
            ("a\0b", "error Leerer oder ungültiger Pfad"),
        ];
        for (endpoint, expected) in table {
            assert_eq!(shape(endpoint), expected, "{endpoint:?}");
        }
        validate_sync_endpoints("/a", "/b").unwrap();
        validate_sync_endpoints("sftp://u@h:22/a", "sftp://u@h:22/b").unwrap();
        validate_sync_endpoints("/a", "sftp://u@h:22/a").unwrap();
        assert!(validate_sync_endpoints("/a", "/a/b").is_err());
        assert!(validate_sync_endpoints("gdrive:///a", "gdrive:///a/b").is_err());
        // The remote detection and the test-only local dispatch keep their answers.
        for remote in [
            "sftp://u@h:22/x",
            "webdav://u@h:443/dav",
            "share://direct/c/G",
        ] {
            assert!(crate::connect::is_remote_url(remote), "{remote}");
        }
        assert!(!crate::connect::is_remote_url("/home/anna"));
        assert!(!crate::vfs::is_remote_root(r"C:\Users"));
        assert!(crate::vfs::is_remote_root("sftp://h/p"));
        assert_eq!(
            crate::vfs::backend_for("/tmp").unwrap().scheme(),
            crate::vfs::Scheme::Local
        );
        assert_eq!(
            crate::connect::local_endpoint_path("/home/anna"),
            Ok(Some("/home/anna".to_string()))
        );
        assert_eq!(
            crate::connect::local_endpoint_path("sftp://u@h:22/x"),
            Ok(None)
        );
    }

    #[test]
    fn vault_task_locator_never_reaches_local_backend() {
        let vault = format!("vault://{SITE}/Docs/a.txt");
        let store = format!("vault-store://{SITE}");
        assert_eq!(shape(&vault), format!("vault {SITE} /Docs/a.txt"));
        assert_eq!(shape(&format!("VAULT://{SITE}")), format!("vault {SITE} /"));
        assert_eq!(shape("vault://Mein Tresor/x"), "vault Mein Tresor /x");
        assert_eq!(shape(&store), format!("vault-store {SITE}"));
        assert!(shape("vault:///x").starts_with("error "));
        assert!(shape(&format!("vault-store://{SITE}/x")).starts_with("error "));
        // Never a local path, never the local backend, never a sync side.
        assert_eq!(crate::connect::local_endpoint_path(&vault), Ok(None));
        assert_eq!(crate::connect::local_endpoint_path(&store), Ok(None));
        for locator in [vault.as_str(), store.as_str()] {
            assert!(crate::vfs::backend_for(locator).is_err(), "{locator}");
            assert!(crate::vfs::is_remote_root(locator), "{locator}");
            assert_eq!(
                validate_sync_endpoints(locator, "/home/anna/x"),
                Err(VAULT_SIDE_REFUSED.to_string())
            );
            assert_eq!(
                validate_sync_endpoints("/home/anna/x", locator),
                Err(VAULT_SIDE_REFUSED.to_string())
            );
        }
        // An unknown site resolves to an error, not to a local folder.
        let unknown = "vault://fedcba9876543210fedcba9876543210/x";
        assert!(crate::connect::resolve_endpoint(unknown).is_err());
        let unknown_store = "vault-store://fedcba9876543210fedcba9876543210";
        assert!(crate::connect::resolve_endpoint(unknown_store).is_err());
    }
}
