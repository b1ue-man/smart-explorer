use super::io_err;
use super::{SftpAuth, SftpBackend, SftpConfig};
use std::io;

/// Parsed `sftp://[user[:password]@]host[:port][/path]`.
struct SftpUrl {
    user: String,
    password: Option<String>,
    host: String,
    port: u16,
    root: String,
}

fn parse_sftp_url(url: &str) -> io::Result<SftpUrl> {
    let rest = url
        .trim()
        .strip_prefix("sftp://")
        .ok_or_else(|| io_err("kein sftp://-URL"))?;
    // authority / path
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let root = if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    };
    // [user[:password]@]host[:port]
    let (userinfo, hostport) = match authority.rfind('@') {
        Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
        None => (None, authority),
    };
    let (user, password) = match userinfo {
        Some(ui) => match ui.find(':') {
            Some(j) => (ui[..j].to_string(), Some(ui[j + 1..].to_string())),
            None => (ui.to_string(), None),
        },
        None => return Err(io_err("SFTP-Benutzername fehlt (sftp://user@host/…)")),
    };
    if user.is_empty() {
        return Err(io_err("SFTP-Benutzername fehlt"));
    }
    let (host, port) = match hostport.rfind(':') {
        Some(k) => {
            let p = hostport[k + 1..]
                .parse::<u16>()
                .map_err(|_| io_err("ungültiger SFTP-Port"))?;
            (hostport[..k].to_string(), p)
        }
        None => (hostport.to_string(), 22),
    };
    if host.is_empty() {
        return Err(io_err("SFTP-Host fehlt"));
    }
    Ok(SftpUrl {
        user,
        password,
        host,
        port,
        root,
    })
}

/// Connect from a `sftp://` URL. A password embedded in the URL is used; without
/// one the caller must go through the Connect dialog (credential store) — wired
/// in the connect-UI step.
pub fn backend_from_url(url: &str) -> io::Result<SftpBackend> {
    let u = parse_sftp_url(url)?;
    let auth = match u.password {
        Some(p) => SftpAuth::Password(p),
        None => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "SFTP-Zugangsdaten erforderlich — bitte den Verbinden-Dialog nutzen",
            ))
        }
    };
    SftpBackend::connect(SftpConfig {
        host: u.host,
        port: u.port,
        user: u.user,
        auth,
        root: u.root,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sftp::metadata::basename;

    #[test]
    fn url_full() {
        let u = parse_sftp_url("sftp://alice:secret@example.com:2222/home/alice").unwrap();
        assert_eq!(u.user, "alice");
        assert_eq!(u.password.as_deref(), Some("secret"));
        assert_eq!(u.host, "example.com");
        assert_eq!(u.port, 2222);
        assert_eq!(u.root, "/home/alice");
    }

    #[test]
    fn url_defaults() {
        let u = parse_sftp_url("sftp://bob@host").unwrap();
        assert_eq!(u.user, "bob");
        assert!(u.password.is_none());
        assert_eq!(u.host, "host");
        assert_eq!(u.port, 22);
        assert_eq!(u.root, "/");

        let u2 = parse_sftp_url("sftp://bob@host/").unwrap();
        assert_eq!(u2.root, "/");
    }

    #[test]
    fn url_errors() {
        assert!(parse_sftp_url("sftp://host/path").is_err()); // no user
        assert!(parse_sftp_url("ftp://u@host").is_err()); // wrong scheme
        assert!(parse_sftp_url("sftp://u@host:notaport/").is_err());
    }

    #[test]
    fn url_without_password_needs_dialog() {
        // backend_from_url must refuse (not connect) when no password is present.
        match backend_from_url("sftp://bob@host/") {
            Err(e) => assert_eq!(e.kind(), io::ErrorKind::PermissionDenied),
            Ok(_) => panic!("should require credentials"),
        }
    }

    #[test]
    fn basename_works() {
        assert_eq!(basename("/home/user/file.txt"), "file.txt");
        assert_eq!(basename("/home/user/"), "user");
        assert_eq!(basename("file"), "file");
    }
}
