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

use super::connection::{connect_stream, parse_ftp_url};
use super::io_adapters::FtpConnection;
use super::writer::FtpWriter;

#[cfg(test)]
use super::connection::FtpUrl;
#[cfg(test)]
use std::time::Duration;

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
    let ftp = connect_stream(&u)?;
    let url = format!(
        "{}://{}@{}:{}{}",
        if u.secure { "ftps" } else { "ftp" },
        u.user,
        u.host,
        u.port,
        u.root
    );
    let reconnect_config = u.clone();
    let reconnect = Arc::new(move || connect_stream(&reconnect_config));
    Ok(FtpBackend {
        conn: FtpConnection::new(ftp, reconnect)?,
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
            .with_stream_read(|stream| stream.list(Some(path)).map_err(io_err))?;
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
            .with_stream_mutation(|stream| stream.rename(src, dst).map_err(io_err))
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.conn
            .with_stream_mutation(|stream| stream.rm(path).map_err(io_err))
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.conn
            .with_stream_mutation(|stream| stream.rmdir(path).map_err(io_err))
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        let absolute = path.starts_with('/');
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        self.conn.with_stream_mutation(|stream| {
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
    use std::io::{BufRead, BufReader};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::thread;

    fn reply(stream: &mut TcpStream, line: &str) {
        stream.write_all(line.as_bytes()).unwrap();
        stream.write_all(b"\r\n").unwrap();
        stream.flush().unwrap();
    }

    fn serve_control_connection(
        mut stream: TcpStream,
        generation: usize,
        kept_alive: &mpsc::Sender<()>,
    ) {
        reply(&mut stream, "220 test ready");
        let reader_stream = stream.try_clone().unwrap();
        let mut reader = BufReader::new(reader_stream);
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let command = line.trim_end_matches(['\r', '\n']);
            match command.split_whitespace().next().unwrap_or_default() {
                "USER" => reply(&mut stream, "331 password required"),
                "PASS" => reply(&mut stream, "230 logged in"),
                "TYPE" => reply(&mut stream, "200 binary"),
                "NOOP" if generation == 0 => {
                    let _ = stream.shutdown(Shutdown::Both);
                    return;
                }
                "NOOP" => {
                    reply(&mut stream, "200 alive");
                    let _ = kept_alive.send(());
                }
                "PWD" => reply(&mut stream, "257 \"/\" is current directory"),
                "QUIT" => {
                    reply(&mut stream, "221 bye");
                    return;
                }
                _ => reply(&mut stream, "500 unsupported"),
            }
        }
    }

    fn serve_suspect_connection(mut stream: TcpStream, generation: usize, mutations: &AtomicUsize) {
        reply(&mut stream, "220 test ready");
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            match line
                .trim_end_matches(['\r', '\n'])
                .split_whitespace()
                .next()
                .unwrap_or_default()
            {
                "USER" => reply(&mut stream, "331 password required"),
                "PASS" => reply(&mut stream, "230 logged in"),
                "TYPE" => reply(&mut stream, "200 binary"),
                "DELE" if generation == 0 => {
                    mutations.fetch_add(1, Ordering::SeqCst);
                    let _ = stream.shutdown(Shutdown::Both);
                    return;
                }
                "PWD" => reply(&mut stream, "257 \"/\" is current directory"),
                _ => reply(&mut stream, "500 unsupported"),
            }
        }
    }

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
    fn idle_ftp_control_channel_is_pinged_and_reconnected() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let accepts = Arc::new(AtomicUsize::new(0));
        let server_accepts = accepts.clone();
        let (kept_alive_tx, kept_alive_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            for generation in 0..2 {
                let (stream, _) = listener.accept().unwrap();
                server_accepts.fetch_add(1, Ordering::SeqCst);
                serve_control_connection(stream, generation, &kept_alive_tx);
            }
        });
        let config = FtpUrl {
            secure: false,
            user: "test".to_string(),
            password: "secret".to_string(),
            host: address.ip().to_string(),
            port: address.port(),
            root: "/".to_string(),
        };
        let stream = connect_stream(&config).unwrap();
        assert_eq!(
            stream.get_ref().read_timeout().unwrap(),
            Some(Duration::from_secs(60))
        );
        assert_eq!(
            stream.get_ref().write_timeout().unwrap(),
            Some(Duration::from_secs(60))
        );
        let reconnect_config = config.clone();
        let reconnect: super::super::io_adapters::FtpReconnect =
            Arc::new(move || connect_stream(&reconnect_config));
        let connection = FtpConnection::new_with_timing(
            stream,
            reconnect,
            Duration::from_millis(20),
            Duration::from_millis(200),
        )
        .unwrap();

        kept_alive_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("replacement connection must receive NOOP");
        assert_eq!(accepts.load(Ordering::SeqCst), 2);
        let pwd = connection
            .with_stream_read(|stream| stream.pwd().map_err(io_err))
            .unwrap();
        assert_eq!(pwd, "/");

        drop(connection);
        server.join().unwrap();
    }

    #[test]
    fn ambiguous_mutation_marks_channel_suspect_and_next_read_reconnects() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mutations = Arc::new(AtomicUsize::new(0));
        let server_mutations = mutations.clone();
        let server = thread::spawn(move || {
            for generation in 0..2 {
                let (stream, _) = listener.accept().unwrap();
                serve_suspect_connection(stream, generation, &server_mutations);
            }
        });
        let config = FtpUrl {
            secure: false,
            user: "test".to_string(),
            password: "secret".to_string(),
            host: address.ip().to_string(),
            port: address.port(),
            root: "/".to_string(),
        };
        let stream = connect_stream(&config).unwrap();
        let reconnect_config = config.clone();
        let reconnect: super::super::io_adapters::FtpReconnect =
            Arc::new(move || connect_stream(&reconnect_config));
        let connection = FtpConnection::new_with_timing(
            stream,
            reconnect,
            Duration::from_secs(60),
            Duration::from_millis(200),
        )
        .unwrap();

        assert!(connection
            .with_stream_mutation(|stream| stream.rm("/committed").map_err(io_err))
            .is_err());
        let pwd = connection
            .with_stream_read(|stream| stream.pwd().map_err(io_err))
            .unwrap();
        assert_eq!(pwd, "/");
        assert_eq!(mutations.load(Ordering::SeqCst), 1);

        drop(connection);
        server.join().unwrap();
    }
}
