//! FTP / FTPS backend (`suppaftp`, blocking) implementing `vfs::Backend`.
//!
//! One `RustlsFtpStream` type carries both plain FTP (`ftp://`) and explicit
//! FTPS (`ftps://` — AUTH TLS after connect). TLS is rustls backed by **ring**
//! (no native-tls / schannel FFI on GNU; see docs/GOTCHAS.md) with bundled
//! webpki-roots. The single control connection is serialized behind a `Mutex`
//! (`parallelism() == 1`).
//!
//! Listings are parsed by suppaftp's `list::File` (posix / dos / mlsx). File I/O
//! buffers whole files in memory (FTP has no seekable streaming copy): download
//! via `retr_as_buffer`, upload via `put_file` on flush/drop.

use crate::vfs::{Backend, Scheme, VfsMeta, VfsResult};
use std::io::{self, Cursor, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use suppaftp::types::FileType;
use suppaftp::{RustlsConnector, RustlsFtpStream};

fn io_err<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
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

// ── URL ──────────────────────────────────────────────────────────────────────

struct FtpUrl {
    secure: bool,
    user: String,
    password: String,
    host: String,
    port: u16,
    root: String,
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
            Some(j) => (ui[..j].to_string(), ui[j + 1..].to_string()),
            None => (ui.to_string(), String::new()),
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
    conn: Arc<Mutex<RustlsFtpStream>>,
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
        conn: Arc::new(Mutex::new(ftp)),
        root: u.root,
        url,
    })
}

impl FtpBackend {
    fn lock(&self) -> io::Result<std::sync::MutexGuard<'_, RustlsFtpStream>> {
        self.conn.lock().map_err(|_| io_err("FTP-Verbindung vergiftet"))
    }
}

impl Backend for FtpBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Ftp
    }
    fn root_display(&self) -> String {
        self.root.clone()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let lines = {
            let mut g = self.lock()?;
            g.list(Some(path)).map_err(io_err)?
        };
        let mut out = Vec::new();
        for line in lines {
            // suppaftp parses unix/dos/mlsx; skip lines it can't read.
            if let Ok(f) = line.parse::<suppaftp::list::File>() {
                let name = f.name().to_string();
                if name == "." || name == ".." {
                    continue;
                }
                out.push(VfsMeta {
                    is_dir: f.is_directory(),
                    is_symlink: f.is_symlink(),
                    size: f.size() as u64,
                    mtime_ms: systime_ms(f.modified()),
                    btime_ms: 0,
                    hidden: name.starts_with('.'),
                    system: false,
                    name,
                    id: None,
                    content_md5: None,
                });
            }
        }
        Ok(out)
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
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{path} nicht gefunden")))
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let buf = {
            let mut g = self.lock()?;
            g.retr_as_buffer(path).map_err(io_err)?
        };
        Ok(Box::new(buf)) // Cursor<Vec<u8>>
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Ok(Box::new(FtpWriter {
            conn: self.conn.clone(),
            path: path.to_string(),
            buf: Vec::new(),
            committed: false,
        }))
    }

    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        let mut g = self.lock()?;
        g.rename(src, dst).map_err(io_err)
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        let mut g = self.lock()?;
        g.rm(path).map_err(io_err)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        let mut g = self.lock()?;
        g.rmdir(path).map_err(io_err)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        let absolute = path.starts_with('/');
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut g = self.lock()?;
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
            let _ = g.mkdir(&cur); // ignore "already exists"
        }
        Ok(())
    }

    fn parallelism(&self) -> usize {
        1 // single control connection
    }
}

// ── buffering writer (one-shot STOR on flush/drop) ───────────────────────────

struct FtpWriter {
    conn: Arc<Mutex<RustlsFtpStream>>,
    path: String,
    buf: Vec<u8>,
    committed: bool,
}

impl FtpWriter {
    fn commit(&mut self) -> io::Result<()> {
        if self.committed {
            return Ok(());
        }
        self.committed = true;
        let data = std::mem::take(&mut self.buf);
        let mut g = self
            .conn
            .lock()
            .map_err(|_| io_err("FTP-Verbindung vergiftet"))?;
        let mut cur = Cursor::new(data);
        g.put_file(&self.path, &mut cur).map(|_| ()).map_err(io_err)
    }
}

impl Write for FtpWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.committed {
            return Err(io_err("Upload bereits abgeschlossen"));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.commit()
    }
}

impl Drop for FtpWriter {
    fn drop(&mut self) {
        let _ = self.commit();
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
    fn rustls_config_builds_with_ring() {
        // Constructing the FTPS client config must not panic (ring provider).
        let _ = rustls_client_config();
    }
}
