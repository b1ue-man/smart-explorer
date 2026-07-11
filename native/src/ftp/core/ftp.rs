//! FTP / FTPS backend (`suppaftp`, blocking) implementing `vfs::Backend`.
//!
//! One `RustlsFtpStream` type carries both plain FTP (`ftp://`) and explicit
//! FTPS (`ftps://` — AUTH TLS after connect). TLS is rustls backed by **ring**
//! (no native-tls / schannel FFI on GNU; see docs/GOTCHAS.md) with bundled
//! webpki-roots. The single control connection is serialized behind a `Mutex`
//! (`parallelism() == 1`).
//!
//! Listings are parsed by suppaftp's `list::File` (posix / dos / mlsx). RETR
//! streams directly from the data connection. Uploads spool to disk and issue a
//! streaming STOR only at the caller's explicit `flush` commit boundary.

use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use suppaftp::types::FileType;
use suppaftp::{RustlsConnector, RustlsFtpStream};

use super::io_adapters::{FtpConnection, FtpWriter};

fn io_err<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::other(e.to_string())
}

fn systime_ms(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    }
}

fn basename(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_string()
}

fn parent_dir(path: &str) -> String {
    let t = path.trim_end_matches('/');
    match t.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => t[..i].to_string(),
    }
}

fn dir_meta(name: String) -> VfsMeta {
    VfsMeta {
        name,
        is_dir: true,
        is_symlink: false,
        size: 0,
        mtime_ms: 0,
        btime_ms: 0,
        hidden: false,
        system: false,
        id: None,
        content_md5: None,
    }
}

fn parse_list_line(line: &str) -> VfsResult<VfsMeta> {
    let file = line.parse::<suppaftp::list::File>().map_err(|error| {
        let preview: String = line.chars().take(160).collect();
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("FTP LIST row could not be parsed ({error}): {preview:?}"),
        )
    })?;
    let name = file.name().to_string();
    crate::vfs::validate_child_name(&name)?;
    Ok(VfsMeta {
        is_dir: file.is_directory(),
        is_symlink: file.is_symlink(),
        size: file.size() as u64,
        mtime_ms: systime_ms(file.modified()),
        btime_ms: 0,
        hidden: name.starts_with('.'),
        system: false,
        name,
        id: None,
        content_md5: None,
    })
}

// ── URL ──────────────────────────────────────────────────────────────────────

struct FtpUrl {
    secure: bool,
    user: String,
    password: String,
    host: String,
    port: u16,
    root: String,
}

fn decode_userinfo(value: &str) -> io::Result<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'%' {
            decoded.push(bytes[index]);
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len() {
            return Err(io_err(
                "unvollstaendige Prozentkodierung in FTP-Zugangsdaten",
            ));
        }
        let high = hex_nibble(bytes[index + 1])
            .ok_or_else(|| io_err("ungueltige Prozentkodierung in FTP-Zugangsdaten"))?;
        let low = hex_nibble(bytes[index + 2])
            .ok_or_else(|| io_err("ungueltige Prozentkodierung in FTP-Zugangsdaten"))?;
        decoded.push((high << 4) | low);
        index += 3;
    }
    String::from_utf8(decoded).map_err(|_| io_err("FTP-Zugangsdaten sind nicht gueltiges UTF-8"))
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn parse_ftp_url(url: &str) -> io::Result<FtpUrl> {
    let u = url.trim();
    let (secure, rest) = if let Some(r) = u.strip_prefix("ftps://") {
        (true, r)
    } else if let Some(r) = u.strip_prefix("ftp://") {
        (false, r)
    } else {
        return Err(io_err("kein ftp(s)://-URL"));
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let root = if path.is_empty() {
        "/".to_string()
    } else {
        path.to_string()
    };
    let (userinfo, hostport) = match authority.rfind('@') {
        Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
        None => (None, authority),
    };
    let (user, password) = match userinfo {
        Some(ui) => match ui.find(':') {
            Some(j) => (decode_userinfo(&ui[..j])?, decode_userinfo(&ui[j + 1..])?),
            None => (decode_userinfo(ui)?, String::new()),
        },
        // Bare ftp://host → anonymous login (the standard FTP convention).
        None => ("anonymous".to_string(), "anonymous@example.com".to_string()),
    };
    let (host, port) = match hostport.rfind(':') {
        Some(k) => {
            let p = hostport[k + 1..]
                .parse::<u16>()
                .map_err(|_| io_err("ungültiger FTP-Port"))?;
            (hostport[..k].to_string(), p)
        }
        None => (hostport.to_string(), 21),
    };
    if host.is_empty() {
        return Err(io_err("FTP-Host fehlt"));
    }
    Ok(FtpUrl {
        secure,
        user,
        password,
        host,
        port,
        root,
    })
}

fn rustls_client_config() -> Arc<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("ring provider supports default protocol versions")
    .with_root_certificates(roots)
    .with_no_client_auth();
    Arc::new(cfg)
}

// ── backend ──────────────────────────────────────────────────────────────────

pub struct FtpBackend {
    conn: Arc<FtpConnection>,
    root: String,
    /// `ftp(s)://user@host:port/root` for UI display (connect-UI step).
    #[allow(dead_code)]
    url: String,
}

/// Connect from an `ftp://` / `ftps://` URL. Plain FTP allows anonymous login,
/// so (unlike SFTP) a bare URL connects without a credential dialog.
pub fn backend_from_url(url: &str) -> io::Result<FtpBackend> {
    let u = parse_ftp_url(url)?;
    let mut ftp = RustlsFtpStream::connect((u.host.as_str(), u.port)).map_err(io_err)?;
    if u.secure {
        let connector = RustlsConnector::from(rustls_client_config());
        ftp = ftp.into_secure(connector, &u.host).map_err(io_err)?;
    }
    ftp.login(&u.user, &u.password).map_err(io_err)?;
    // Binary mode — ASCII mode would corrupt non-text transfers.
    ftp.transfer_type(FileType::Binary).map_err(io_err)?;
    let url = format!(
        "{}://{}@{}:{}{}",
        if u.secure { "ftps" } else { "ftp" },
        u.user,
        u.host,
        u.port,
        u.root
    );
    Ok(FtpBackend {
        conn: Arc::new(FtpConnection::new(ftp)),
        root: u.root,
        url,
    })
}

impl Backend for FtpBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Ftp
    }
    fn root_display(&self) -> String {
        self.root.clone()
    }
    fn state_identity(&self) -> String {
        self.url.clone()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let lines = self
            .conn
            .with_stream(|stream| stream.list(Some(path)).map_err(io_err))?;
        lines
            .into_iter()
            .map(|line| parse_list_line(&line))
            .collect()
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let base = basename(path);
        if path == "/" || base.is_empty() {
            return Ok(dir_meta(if base.is_empty() {
                "/".to_string()
            } else {
                base
            }));
        }
        // FTP has no stat: list the parent and find the entry.
        let parent = parent_dir(path);
        self.list_dir(&parent)?
            .into_iter()
            .find(|e| e.name == base)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("{path} nicht gefunden"))
            })
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        Ok(Box::new(self.conn.open_reader(path)?))
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(FtpWriter::new(
            self.conn.clone(),
            path.to_string(),
        )?))
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.conn
            .with_stream(|stream| stream.rename(src, dst).map_err(io_err))
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.conn
            .with_stream(|stream| stream.rm(path).map_err(io_err))
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.conn
            .with_stream(|stream| stream.rmdir(path).map_err(io_err))
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        let absolute = path.starts_with('/');
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        self.conn.with_stream(|stream| {
            let original = stream.pwd().map_err(io_err)?;
            let mut cur = String::new();
            for part in parts {
                if cur.is_empty() {
                    if absolute {
                        cur.push('/');
                    }
                } else {
                    cur.push('/');
                }
                cur.push_str(part);
                if let Err(mkdir_error) = stream.mkdir(&cur) {
                    stream.cwd(&cur).map_err(|verify_error| {
                        io::Error::other(format!(
                            "FTP mkdir failed for {cur}: {mkdir_error}; existing-directory verification failed: {verify_error}"
                        ))
                    })?;
                    stream.cwd(&original).map_err(io_err)?;
                }
            }
            Ok(())
        })
    }

    fn parallelism(&self) -> usize {
        1 // single control connection
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_plain_with_creds() {
        let u = parse_ftp_url("ftp://bob:pw@host:2121/pub/data").unwrap();
        assert!(!u.secure);
        assert_eq!(u.user, "bob");
        assert_eq!(u.password, "pw");
        assert_eq!(u.host, "host");
        assert_eq!(u.port, 2121);
        assert_eq!(u.root, "/pub/data");
    }

    #[test]
    fn url_decodes_encoded_userinfo_exactly() {
        let u = parse_ftp_url("ftp://domain%40alice:p%3Aa%20ss%25%40word@host:21/").unwrap();
        assert_eq!(u.user, "domain@alice");
        assert_eq!(u.password, "p:a ss%@word");
        assert!(parse_ftp_url("ftp://user:%GG@host/").is_err());
        assert!(parse_ftp_url("ftp://user:%A@host/").is_err());
    }

    #[test]
    fn url_ftps_default_port() {
        let u = parse_ftp_url("ftps://alice@example.com/").unwrap();
        assert!(u.secure);
        assert_eq!(u.user, "alice");
        assert_eq!(u.port, 21);
        assert_eq!(u.root, "/");
    }

    #[test]
    fn url_anonymous() {
        let u = parse_ftp_url("ftp://ftp.example.com/pub").unwrap();
        assert_eq!(u.user, "anonymous");
        assert!(!u.password.is_empty());
        assert_eq!(u.host, "ftp.example.com");
        assert_eq!(u.port, 21);
        assert_eq!(u.root, "/pub");
    }

    #[test]
    fn url_errors() {
        assert!(parse_ftp_url("sftp://u@host").is_err());
        assert!(parse_ftp_url("ftp://u@host:bad/").is_err());
    }

    #[test]
    fn path_helpers() {
        assert_eq!(basename("/a/b/c.txt"), "c.txt");
        assert_eq!(parent_dir("/a/b/c.txt"), "/a/b");
        assert_eq!(parent_dir("/a"), "/");
        assert_eq!(parent_dir("/"), "/");
    }

    #[test]
    fn list_rows_are_fail_closed() {
        let entry = parse_list_line("-rw-rw-r-- 1 0 1 8192 Nov 5 2018 report.txt").unwrap();
        assert_eq!(entry.name, "report.txt");
        assert_eq!(entry.size, 8192);

        assert_eq!(
            parse_list_line("this server row is not parseable")
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert!(parse_list_line("-rw-rw-r-- 1 0 1 1 Nov 5 2018 ..").is_err());
        assert!(parse_list_line("-rw-rw-r-- 1 0 1 1 Nov 5 2018 ..\\escape").is_err());
    }

    #[test]
    fn rustls_config_builds_with_ring() {
        // Constructing the FTPS client config must not panic (ring provider).
        let _ = rustls_client_config();
    }
}
