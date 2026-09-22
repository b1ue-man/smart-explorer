//! Application locators keep their historical literal backend paths. In
//! particular, `%20` names are not decoded into spaces during a sync reopen.
use crate::creds::{Protocol, SavedConnection};

use super::endpoint::parse_remote_url;

pub(crate) enum EndpointSpec {
    Local(String),
    Saved(String),
    Drive(String),
    Peer(crate::share::PeerOpenTarget, String),
}

impl EndpointSpec {
    pub(crate) fn parse(endpoint: &str) -> Result<Self, String> {
        if endpoint.trim().is_empty() || endpoint.contains('\0') {
            return Err("Leerer oder ungültiger Pfad".into());
        }
        if endpoint.starts_with('/') || endpoint.starts_with('\\')
            || (endpoint.as_bytes().get(1) == Some(&b':')
                && endpoint.as_bytes()[0].is_ascii_alphabetic())
        {
            return Ok(Self::Local(local_root(endpoint)));
        }
        let Some((scheme, rest)) = endpoint.split_once("://") else {
            return Ok(Self::Local(local_root(endpoint)));
        };
        // A colon within an absolute filesystem path is still a filename.
        if !scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
            return Ok(Self::Local(local_root(endpoint)));
        }
        let scheme = scheme.to_ascii_lowercase();
        match scheme.as_str() {
            "gdrive" => Ok(Self::Drive(format!("/{}", rest.trim_start_matches('/')))),
            "share" => crate::share::PeerOpenTarget::from_endpoint(&format!("share://{rest}"))
                .map(|(target, path)| Self::Peer(target, path))
                .ok_or_else(|| "Ungültige Share-Adresse".into()),
            "sftp" | "ftp" | "ftps" | "webdav" => {
                if rest.is_empty() {
                    return Err("Ungültige Remote-Adresse".into());
                }
                Ok(Self::Saved(format!("{scheme}://{rest}")))
            }
            _ => Err(format!("Nicht unterstütztes Pfadprotokoll: {scheme}")),
        }
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
    if drive && root.len() == 2 { format!("{root}/") } else { root }
}

/// Choose credentials within one authority by the most specific saved root.
/// Exact legacy prefixes also preserve usernames containing URL delimiters.
pub(crate) fn saved_location<'a>(
    connections: &'a [SavedConnection],
    endpoint: &str,
) -> Option<(&'a SavedConnection, String)> {
    let parsed = parse_remote_url(endpoint);
    connections.iter().enumerate().filter(|(_, connection)| connection.protocol.is_url()).filter_map(|(index, connection)| {
        let prefix = format!("{}://{}@{}:{}", connection.protocol.as_str(),
            connection.user, connection.host, connection.port);
        let direct = endpoint.strip_prefix(&prefix)
            .filter(|path| path.is_empty() || path.starts_with('/'))
            .map(|path| if path.is_empty() { "/".into() } else { path.to_string() });
        let path = direct.or_else(|| {
            let (protocol, user, host, port, path) = parsed.as_ref()?;
            (*protocol == connection.protocol && *user == connection.user
                && hosts_equal(host, &connection.host) && *port == connection.port)
                .then(|| path.clone())
        })?;
        let root = connection.root.trim_end_matches('/');
        let score = if paths_overlap_one_way(root, &path) { root.len() + 1 } else { 0 };
        Some((connection, path, (score, std::cmp::Reverse(index))))
    }).max_by_key(|(_, _, score)| *score).map(|(connection, path, _)| (connection, path))
}

fn hosts_equal(a: &str, b: &str) -> bool {
    a.trim_matches(['[', ']']).eq_ignore_ascii_case(b.trim_matches(['[', ']']))
}

/// Pure endpoint validation shared by the editor, persistence and daemon.
pub(crate) fn validate_sync_endpoints(source: &str, target: &str) -> Result<(), String> {
    EndpointSpec::parse(source)?;
    EndpointSpec::parse(target)?;
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
            let windows = path.starts_with("//") || path.as_bytes().get(1) == Some(&b':');
            let path = if windows { path.to_lowercase() } else { path };
            ("local".into(), path_key(&path))
        }
        Ok(EndpointSpec::Drive(path)) => ("gdrive".into(), path_key(&path)),
        Ok(EndpointSpec::Peer(target, path)) => (target.endpoint_prefix(), path_key(&path)),
        Ok(EndpointSpec::Saved(url)) => {
            if let Some((proto, user, host, port, path)) = parse_remote_url(&url) {
                (format!("{}://{}@{}:{}", proto.as_str(), user,
                    host.trim_matches(['[', ']']).to_ascii_lowercase(), port), path_key(&path))
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
    parent == child || child.strip_prefix(parent).is_some_and(|rest| rest.starts_with('/'))
}

fn path_key(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "." => {}
            ".." if parts.last().is_some_and(|part| !part.is_empty() && *part != "..") => { parts.pop(); }
            _ => parts.push(part),
        }
    }
    parts.join("/").trim_end_matches('/').to_string()
}

pub(super) fn parse_host_port(hostport: &str, protocol: Protocol) -> Option<(String, u16)> {
    let (host, port) = if let Some(rest) = hostport.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        host.parse::<std::net::Ipv6Addr>().ok()?;
        let port = if tail.is_empty() { protocol.default_port() } else { tail.strip_prefix(':')?.parse().ok()? };
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
    if host.is_empty() || host.contains([' ', '\\', '?', '#']) || port == 0 { None } else { Some((host, port)) }
}
