//! Content digests through the background service's wrappers: the rooted
//! mount backend maps and confines the path exactly as for a read and
//! forwards to the backend it wraps, an inner `None` stays `None`, the
//! service answers `query::FILE_DIGEST` with 1 and serves a one-file
//! SHA-256 walk through `vfs::content_digest` of the served backend, and the
//! mount proxy reaches all of it. No step reads the file to get a digest.
use super::super::backend_server::serve_backend;
use super::super::ipc_protocol::MountBackendCapabilities;
use super::super::mount_proxy;
use super::super::rooted_backend::RootedBackend;
use crate::agent::AgentBackend;
use crate::agent_proto::{digest, query, read_frame, write_frame, Frame};
use crate::mount::{BackendRoot, MountMode, MountRootSecurity};
use crate::vfs::{
    self as vfs, Backend, BackendExtensions, BackendHandle, ContentDigest, VfsMeta, VfsResult,
};
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

fn abc_digest() -> ContentDigest {
    ContentDigest {
        sha256: ABC_SHA256.into(),
        size: 3,
    }
}

/// A root-confined remote stand-in: `/root/file` is a 3-byte file whose
/// digest the "host" knows (or not); every content read is counted and
/// refused, so a test sees any read-back at once.
struct DigestProbe {
    digest: Option<ContentDigest>,
    digested: Mutex<Vec<String>>,
    reads: AtomicUsize,
}

impl DigestProbe {
    fn new(digest: Option<ContentDigest>) -> Arc<Self> {
        Arc::new(Self {
            digest,
            digested: Mutex::new(Vec::new()),
            reads: AtomicUsize::new(0),
        })
    }

    fn digested(&self) -> Vec<String> {
        self.digested.lock().unwrap().clone()
    }
}

fn refused<T>() -> VfsResult<T> {
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        "probe refuses this",
    ))
}

impl Backend for DigestProbe {
    fn scheme(&self) -> vfs::Scheme {
        vfs::Scheme::Sftp
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        match path {
            "/" | "/root" => Ok(VfsMeta {
                name: "root".into(),
                is_dir: true,
                ..Default::default()
            }),
            "/file" | "/root/file" => Ok(VfsMeta {
                name: "file".into(),
                size: 3,
                mtime_ms: 7,
                ..Default::default()
            }),
            _ => Err(io::ErrorKind::NotFound.into()),
        }
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        match path {
            "/" | "/root" => Ok(vec![self.stat("/root/file")?]),
            _ => Err(io::ErrorKind::NotFound.into()),
        }
    }
    fn open_read(&self, _: &str) -> VfsResult<Box<dyn Read + Send>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        refused()
    }
    fn open_read_at(
        &self,
        _: &str,
        _: Option<&str>,
        _: u64,
    ) -> VfsResult<Option<Box<dyn Read + Send>>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        refused()
    }
    fn open_write(&self, _: &str) -> VfsResult<Box<dyn Write + Send>> {
        refused()
    }
    fn rename(&self, _: &str, _: &str) -> VfsResult<()> {
        refused()
    }
    fn remove_file(&self, _: &str) -> VfsResult<()> {
        refused()
    }
    fn remove_dir(&self, _: &str) -> VfsResult<()> {
        refused()
    }
    fn mkdir_all(&self, _: &str) -> VfsResult<()> {
        refused()
    }
    fn case_sensitive_paths(&self, _: &str) -> bool {
        true
    }
    fn root_confinement(&self, _: &str) -> vfs::RootConfinement {
        vfs::RootConfinement::Enforced
    }
    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(self)
    }
}

impl BackendExtensions for DigestProbe {
    fn content_digest(&self, path: &str, _: &AtomicBool) -> VfsResult<Option<ContentDigest>> {
        self.digested.lock().unwrap().push(path.into());
        Ok(self.digest.clone())
    }
}

fn rooted(probe: &Arc<DigestProbe>) -> BackendHandle {
    let inner: BackendHandle = probe.clone();
    RootedBackend::open_windows(
        inner,
        &BackendRoot::parse("/root").unwrap(),
        MountMode::ReadOnly,
        MountRootSecurity::Enforced,
    )
    .unwrap()
}

fn digest_of(backend: &dyn Backend, path: &str) -> VfsResult<Option<ContentDigest>> {
    vfs::content_digest(backend, path, &AtomicBool::new(false))
}

/// The service serving `backend`, reached by the GUI's `AgentBackend`.
struct Served {
    client: AgentBackend,
    shutdown: TcpStream,
    server: JoinHandle<()>,
}

impl Served {
    fn new(backend: BackendHandle) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            let read = socket.try_clone().unwrap();
            let _ = serve_backend(read, socket, backend);
        });
        let stream = TcpStream::connect(address).unwrap();
        let shutdown = stream.try_clone().unwrap();
        let read: Box<dyn Read + Send> = Box::new(stream.try_clone().unwrap());
        let write: Box<dyn Write + Send> = Box::new(stream);
        let inner: BackendHandle = Arc::new(vfs::LocalBackend::new("/"));
        Self {
            client: AgentBackend::from_streams(read, write, inner).unwrap(),
            shutdown,
            server,
        }
    }

    fn finish(self) {
        drop(self.client);
        let _ = self.shutdown.shutdown(Shutdown::Both);
        self.server.join().unwrap();
    }
}

fn temp_dir(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "se_worker_digest_{label}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// Every reply frame of request `id` up to its `End` or `Err`.
fn replies(client: &mut TcpStream, id: u64) -> Vec<Frame> {
    let mut frames = Vec::new();
    while let Some((reply_id, frame)) = read_frame(client).unwrap() {
        assert_eq!(reply_id, id);
        let last = matches!(frame, Frame::End | Frame::Err(_));
        frames.push(frame);
        if last {
            break;
        }
    }
    frames
}

/// `(size, digest)` of a file `HashEntry` named `wanted`, if `frame` is one.
fn file_entry<'a>(frame: &'a Frame, wanted: &str) -> Option<(u64, &'a str)> {
    match frame {
        Frame::HashEntry {
            rel,
            is_dir: false,
            size,
            md5: Some(sha),
            ..
        } if rel == wanted => Some((*size, sha.as_str())),
        _ => None,
    }
}

#[test]
fn screen_metadata_runtime_task_rooted_backend_confines_and_forwards_digest() {
    let probe = DigestProbe::new(Some(abc_digest()));
    let backend = rooted(&probe);
    assert_eq!(digest_of(&*backend, "/file").unwrap(), Some(abc_digest()));
    // Mapped beneath the selected root exactly like `open_read`.
    assert_eq!(probe.digested(), vec!["/root/file".to_string()]);
    for path in ["/../file", "file", "/missing"] {
        assert!(digest_of(&*backend, path).is_err(), "{path} was hashed");
    }
    assert_eq!(
        probe.digested().len(),
        1,
        "a refused path reached the inner backend"
    );
    assert_eq!(probe.reads.load(Ordering::SeqCst), 0);
}

#[test]
fn screen_metadata_runtime_task_rooted_backend_keeps_inner_none() {
    let probe = DigestProbe::new(None);
    let backend = rooted(&probe);
    assert_eq!(digest_of(&*backend, "/file").unwrap(), None);
    assert_eq!(probe.digested(), vec!["/root/file".to_string()]);
    assert_eq!(probe.reads.load(Ordering::SeqCst), 0);
}

#[test]
fn screen_metadata_runtime_task_worker_answers_file_digest_and_keeps_folder_walks() {
    let base = temp_dir("raw");
    std::fs::write(base.join("abc.txt"), b"abc").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let read = socket.try_clone().unwrap();
        let backend: BackendHandle = Arc::new(vfs::LocalBackend::new("/"));
        let _ = serve_backend(read, socket, backend);
    });
    let mut client = TcpStream::connect(address).unwrap();
    let question = Frame::Query {
        kind: query::FILE_DIGEST,
        path: "/".into(),
    };
    write_frame(&mut client, 3, &question).unwrap();
    assert_eq!(
        read_frame(&mut client).unwrap(),
        Some((3, Frame::Answer(1)))
    );
    // One regular file: one root entry with its SHA-256, then `End`.
    let file = Frame::WalkHashed2 {
        root: base.join("abc.txt").to_string_lossy().into_owned(),
        algorithm: digest::SHA256,
        min_bytes: 0,
    };
    write_frame(&mut client, 4, &file).unwrap();
    let frames = replies(&mut client, 4);
    assert_eq!(frames.len(), 2, "{frames:?}");
    assert_eq!(
        file_entry(&frames[0], ""),
        Some((3, ABC_SHA256)),
        "{frames:?}"
    );
    assert_eq!(frames[1], Frame::End);
    // A folder root keeps the existing SHA-256 folder walk.
    let folder = Frame::WalkHashed2 {
        root: base.to_string_lossy().into_owned(),
        algorithm: digest::SHA256,
        min_bytes: 0,
    };
    write_frame(&mut client, 5, &folder).unwrap();
    let frames = replies(&mut client, 5);
    assert_eq!(frames.last(), Some(&Frame::End), "{frames:?}");
    let mut found = None;
    for frame in &frames {
        found = found.or(file_entry(frame, "abc.txt"));
    }
    assert_eq!(found, Some((3, ABC_SHA256)), "{frames:?}");
    client.shutdown(Shutdown::Both).unwrap();
    drop(client);
    server.join().unwrap();
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn screen_metadata_runtime_task_worker_serves_one_file_digest_from_local_backend() {
    let base = temp_dir("client");
    let file = base.join("abc.txt");
    std::fs::write(&file, b"abc").unwrap();
    let served = Served::new(Arc::new(vfs::LocalBackend::new("/")));
    let path = file.to_string_lossy().into_owned();
    let found = digest_of(&served.client, &path).unwrap();
    assert_eq!(found, Some(abc_digest()));
    // A missing file is an error, never a digest or `None`.
    let gone = base.join("gone.txt").to_string_lossy().into_owned();
    assert!(digest_of(&served.client, &gone).is_err());
    served.finish();
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn screen_metadata_runtime_task_worker_backend_without_digest_yields_none_without_reading() {
    let probe = DigestProbe::new(None);
    let served = Served::new(probe.clone());
    assert_eq!(digest_of(&served.client, "/root/file").unwrap(), None);
    assert_eq!(probe.digested(), vec!["/root/file".to_string()]);
    assert_eq!(probe.reads.load(Ordering::SeqCst), 0);
    served.finish();
}

#[test]
fn screen_metadata_runtime_task_mount_proxy_reaches_the_rooted_worker_digest() {
    let probe = DigestProbe::new(Some(abc_digest()));
    let backend = rooted(&probe);
    let capabilities = MountBackendCapabilities::from_backend(&backend);
    let served = Served::new(backend);
    let Served {
        client,
        shutdown,
        server,
    } = served;
    let proxy = mount_proxy::wrap(Arc::new(client), capabilities);
    assert_eq!(digest_of(&*proxy, "/file").unwrap(), Some(abc_digest()));
    assert!(digest_of(&*proxy, "/../file").is_err());
    assert_eq!(probe.digested(), vec!["/root/file".to_string()]);
    assert_eq!(probe.reads.load(Ordering::SeqCst), 0);
    drop(proxy);
    let _ = shutdown.shutdown(Shutdown::Both);
    server.join().unwrap();
}
