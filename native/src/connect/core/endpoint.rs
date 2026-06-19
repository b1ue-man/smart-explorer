use crate::creds::{Protocol, SavedConnection};

/// Normalize a remote root path to an absolute path.
pub(super) fn norm_root(r: &str) -> String {
    let t = r.trim();
    if t.is_empty() {
        "/".to_string()
    } else if t.starts_with('/') {
        t.to_string()
    } else {
        format!("/{t}")
    }
}

/// Percent-encode a URL userinfo component (FTP URLs).
pub(super) fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// `proto://user@host:port` for URL protocols (sftp/ftp/ftps/webdav); `None` for
/// a local share. The stable per-connection prefix used to key favourites/prefs.
pub(super) fn ep_prefix(form: &crate::connect::ConnectForm, port: u16) -> Option<String> {
    if form.protocol.is_url() {
        Some(format!(
            "{}://{}@{}:{}",
            form.protocol.as_str(),
            form.user.trim(),
            form.host.trim(),
            port
        ))
    } else {
        None
    }
}

/// Split a remote endpoint URL into its matching saved connection + the path
/// part, so a favourite/endpoint can be re-opened.
pub fn saved_and_path(url: &str) -> Option<(SavedConnection, String)> {
    let (proto, user, host, port, path) = parse_remote_url(url)?;
    let c = crate::creds::load_connections()
        .into_iter()
        .find(|c| c.protocol == proto && c.user == user && c.host == host && c.port == port)?;
    Some((c, path))
}

/// Is this endpoint a remote URL (`sftp://...`, `ftp://...`, `ftps://...`,
/// `webdav://...`) rather than a local/UNC path? Used by the sync runner and the
/// in-app picker to decide whether a saved connection must be re-opened.
pub fn is_remote_url(s: &str) -> bool {
    let s = s.trim();
    ["sftp://", "ftp://", "ftps://", "webdav://", "gdrive://"]
        .iter()
        .any(|p| s.starts_with(p))
}

/// Build the `gdrive://` endpoint string for a chosen Drive folder.
#[allow(dead_code)]
pub fn gdrive_endpoint(path: &str) -> String {
    let p = path.trim_start_matches('/');
    format!("gdrive:///{}", p)
}

/// Parse `proto://user@host:port/path` -> its parts (path keeps its leading `/`).
pub(super) fn parse_remote_url(s: &str) -> Option<(Protocol, String, String, u16, String)> {
    let s = s.trim();
    let (scheme, rest) = s.split_once("://")?;
    let proto = Protocol::parse(scheme)?;
    // rest = user@host:port/path  (path optional)
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, "/".to_string()),
    };
    let (user, hostport) = match authority.rsplit_once('@') {
        Some((u, hp)) => (u.to_string(), hp),
        None => (String::new(), authority),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse().unwrap_or_else(|_| proto.default_port()),
        ),
        None => (hostport.to_string(), proto.default_port()),
    };
    Some((
        proto,
        user,
        host,
        port,
        if path.is_empty() { "/".into() } else { path },
    ))
}

/// Build the saved-connection-backed endpoint URL for a chosen remote folder.
#[allow(dead_code)]
pub fn remote_endpoint(c: &SavedConnection, path: &str) -> String {
    let p = if path.is_empty() { "/" } else { path };
    format!(
        "{}://{}@{}:{}{}",
        c.protocol.as_str(),
        c.user,
        c.host,
        c.port,
        p
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::creds::AuthKind;

    #[test]
    fn remote_url_detection_and_parse() {
        assert!(is_remote_url("sftp://u@h:22/x"));
        assert!(is_remote_url("webdav://u@h:443/dav"));
        assert!(!is_remote_url("C:/local"));
        assert!(!is_remote_url(r"\\srv\share"));
        let (p, u, h, port, path) =
            parse_remote_url("sftp://bob@example.com:2222/home/bob").unwrap();
        assert_eq!(p, Protocol::Sftp);
        assert_eq!(u, "bob");
        assert_eq!(h, "example.com");
        assert_eq!(port, 2222);
        assert_eq!(path, "/home/bob");
    }

    #[test]
    fn remote_endpoint_builds_url() {
        let c = SavedConnection {
            protocol: Protocol::Sftp,
            host: "h".into(),
            port: 22,
            user: "u".into(),
            auth: AuthKind::Password,
            root: "/".into(),
            label: String::new(),
            use_agent: false,
        };
        assert_eq!(remote_endpoint(&c, "/data"), "sftp://u@h:22/data");
    }

    #[test]
    fn norm_root_rules() {
        assert_eq!(norm_root(""), "/");
        assert_eq!(norm_root("home/u"), "/home/u");
        assert_eq!(norm_root("/srv"), "/srv");
        assert_eq!(norm_root("  /x  "), "/x");
    }

    #[test]
    fn enc_userinfo() {
        assert_eq!(enc("user"), "user");
        assert_eq!(enc("a@b:c/d"), "a%40b%3Ac%2Fd");
    }
}
