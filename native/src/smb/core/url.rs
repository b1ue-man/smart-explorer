//! Pure parsing of SMB endpoints and backend paths. A backend path is
//! `/<share>/<path in the share>`; `/` itself is the server level above the
//! shares.
use super::{SmbBackend, SmbConfig};
use std::io;

pub(super) const DEFAULT_PORT: u16 = 445;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

/// Parsed `smb://[user[:password]@]host[:port][/share/path]`.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct SmbUrl {
    pub(super) user: String,
    pub(super) password: Option<String>,
    pub(super) host: String,
    pub(super) port: u16,
    pub(super) root: String,
}

pub(super) fn parse_smb_url(url: &str) -> io::Result<SmbUrl> {
    let url = url.trim();
    let rest = url
        .get(..6)
        .filter(|scheme| scheme.eq_ignore_ascii_case("smb://"))
        .and_then(|_| url.get(6..))
        .ok_or_else(|| invalid("kein smb://-URL"))?;
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, "/"),
    };
    let (userinfo, hostport) = match authority.rfind('@') {
        Some(index) => (Some(&authority[..index]), &authority[index + 1..]),
        None => (None, authority),
    };
    let (user, password) = match userinfo {
        Some(info) => match info.split_once(':') {
            Some((user, password)) => (user.to_string(), Some(password.to_string())),
            None => (info.to_string(), None),
        },
        None => (String::new(), None),
    };
    let (host, port) = split_host_port(hostport)?;
    Ok(SmbUrl {
        user,
        password,
        host,
        port,
        root: normalize_root(path),
    })
}

/// `host`, `host:port`, `[v6]`, `[v6]:port` or a bare IPv6 literal.
fn split_host_port(hostport: &str) -> io::Result<(String, u16)> {
    let (host, port) = if let Some(rest) = hostport.strip_prefix('[') {
        let (host, tail) = rest
            .split_once(']')
            .ok_or_else(|| invalid("ungültige IPv6-Adresse"))?;
        match tail.strip_prefix(':') {
            Some(port) => (host, Some(port)),
            None if tail.is_empty() => (host, None),
            None => return Err(invalid("ungültige SMB-Adresse")),
        }
    } else if hostport.matches(':').count() > 1 {
        (hostport, None)
    } else {
        match hostport.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (hostport, None),
        }
    };
    let port = match port {
        Some(port) => port
            .parse::<u16>()
            .ok()
            .filter(|port| *port > 0)
            .ok_or_else(|| invalid("ungültiger SMB-Port"))?,
        None => DEFAULT_PORT,
    };
    if host.is_empty() {
        return Err(invalid("SMB-Host fehlt"));
    }
    Ok((host.to_string(), port))
}

/// Absolute, without empty segments or a trailing slash.
fn normalize_root(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    format!("/{}", parts.join("/"))
}

/// A backend path below the server level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SmbPath {
    pub(super) share: String,
    /// `/`-separated path inside the share; empty for the share root.
    pub(super) rel: String,
}

/// `None` for `/` (the server level); `..` is refused rather than resolved.
pub(super) fn split_path(path: &str) -> io::Result<Option<SmbPath>> {
    let mut parts = Vec::new();
    for part in path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
    {
        if part == ".." {
            return Err(invalid(format!("Ungültiger SMB-Pfad: {path}")));
        }
        parts.push(part);
    }
    let Some((share, rest)) = parts.split_first() else {
        return Ok(None);
    };
    if share.contains('\\') {
        return Err(invalid(format!("Ungültiger Freigabename: {share}")));
    }
    Ok(Some(SmbPath {
        share: share.to_string(),
        rel: rest.join("/"),
    }))
}

/// A path that names an entry inside a share (not the server or share root).
pub(super) fn entry_path(path: &str) -> io::Result<SmbPath> {
    match split_path(path)? {
        Some(target) if !target.rel.is_empty() => Ok(target),
        Some(target) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "Die SMB-Freigabe „{}“ selbst kann nicht verändert werden",
                target.share
            ),
        )),
        None => Err(invalid("Die SMB-Serverebene ist kein Dateipfad")),
    }
}

/// Both ends of a rename; SMB renames only within one share.
pub(super) fn rename_paths(from: &str, to: &str) -> io::Result<(SmbPath, SmbPath)> {
    let (from, to) = (entry_path(from)?, entry_path(to)?);
    if !from.share.eq_ignore_ascii_case(&to.share) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Umbenennen oder Verschieben zwischen verschiedenen SMB-Freigaben wird nicht unterstützt",
        ));
    }
    Ok((from, to))
}

/// `DOMAIN\user` → (`DOMAIN`, `user`); any other name has no domain (NTLM
/// also accepts `user@realm` as the user name itself).
pub(super) fn split_domain_user(user: &str) -> (String, String) {
    match user.split_once('\\') {
        Some((domain, name)) => (domain.to_string(), name.to_string()),
        None => (String::new(), user.to_string()),
    }
}

/// The `host:port` address smb2 dials; IPv6 literals are bracketed.
pub(super) fn server_addr(host: &str, port: u16) -> String {
    if !host.starts_with('[') && host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// The host half of a `host:port` address, the way smb2 derives it
/// (`crate::client::host_of` is private): a bracketed IPv6 literal ends at
/// `]`, otherwise only a numeric port after the last `:` is cut off.
pub(super) fn host_of(addr: &str) -> &str {
    if let Some(host) = addr
        .strip_prefix('[')
        .and_then(|rest| rest.split_once(']'))
        .map(|(host, _)| host)
    {
        return host;
    }
    match addr.rsplit_once(':') {
        Some((host, port)) if port.parse::<u16>().is_ok() => host,
        _ => addr,
    }
}

/// Whether a saved SMB root names a share (`/<share>/…`).
pub fn root_has_share(root: &str) -> bool {
    matches!(split_path(root), Ok(Some(_)))
}

/// The root to open a saved SMB connection at: `path` when it names a share,
/// otherwise the saved root (the server level above the shares has no
/// session of its own).
pub fn root_with_share(path: &str, saved_root: &str) -> String {
    if root_has_share(path) {
        path.to_string()
    } else {
        normalize_root(saved_root)
    }
}

/// Connect from a `smb://` URL. A password embedded in the URL is used;
/// without one the caller must go through a saved connection.
pub fn backend_from_url(url: &str) -> io::Result<SmbBackend> {
    let parsed = parse_smb_url(url)?;
    let Some(password) = parsed.password else {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "SMB-Zugangsdaten erforderlich — bitte eine gespeicherte Verbindung nutzen",
        ));
    };
    SmbBackend::connect(SmbConfig {
        host: parsed.host,
        port: parsed.port,
        user: parsed.user,
        password,
        root: parsed.root,
    })
}
