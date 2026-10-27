//! Existing provider paths remain usable without restart-stable identities.
use super::{
    test_backend::{fwd, job, run, write},
    *,
};
use crate::vfs::{Backend, BackendExtensions, LocalBackend, Scheme, VfsMeta};
use std::{
    fs,
    io::{self, Read, Write},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};

pub(super) struct Limited {
    pub inner: LocalBackend,
    pub identity: String,
    pub read_error: Option<io::ErrorKind>,
    pub lost_directory: AtomicBool,
    pub lost_publication: AtomicBool,
    pub creates: AtomicUsize,
    pub promotes: AtomicUsize,
    pub proofs: bool,
    pub refuse_reserved: bool,
    pub reserved_renames: AtomicUsize,
    pub mutate_stage: AtomicUsize,
    pub refuse_listing: AtomicBool,
}
impl Limited {
    pub fn new(label: &str) -> Self {
        Self {
            inner: LocalBackend::new("/"),
            identity: super::test_backend::unique(label),
            read_error: None,
            lost_directory: AtomicBool::new(false),
            lost_publication: AtomicBool::new(false),
            creates: AtomicUsize::new(0),
            promotes: AtomicUsize::new(0),
            proofs: false,
            refuse_reserved: false,
            reserved_renames: AtomicUsize::new(0),
            mutate_stage: AtomicUsize::new(0),
            refuse_listing: AtomicBool::new(false),
        }
    }
}
impl Backend for Limited {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }
    fn root_display(&self) -> String {
        self.identity.clone()
    }
    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(self)
    }
    fn list_dir(&self, path: &str) -> io::Result<Vec<VfsMeta>> {
        if self.refuse_listing.swap(false, Ordering::SeqCst) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "listing initially refused",
            ));
        }
        self.inner.list_dir(path)
    }
    fn stat(&self, path: &str) -> io::Result<VfsMeta> {
        if path.contains(".se-upload-") && std::path::Path::new(path).is_file() {
            match self.mutate_stage.swap(0, Ordering::SeqCst) {
                1 => fs::remove_file(path)?,
                2 => fs::remove_file(path).and_then(|()| fs::create_dir(path))?,
                3 => fs::write(path, b"changed size after STOR")?,
                _ => {}
            }
        }
        self.inner.stat(path)
    }
    fn open_read(&self, path: &str) -> io::Result<Box<dyn Read + Send>> {
        if let Some(kind) = self.read_error {
            return Err(io::Error::new(kind, "fixture target read refused"));
        }
        self.inner.open_read(path)
    }
    fn open_write(&self, _: &str) -> io::Result<Box<dyn Write + Send>> {
        Err(io::Error::other("replacing writer forbidden"))
    }
    fn open_write_new(&self, path: &str) -> io::Result<Box<dyn Write + Send>> {
        self.inner.open_write_new(path)
    }
    fn promote_copy_stage(&self, source: &str, destination: &str) -> io::Result<()> {
        self.promotes.fetch_add(1, Ordering::SeqCst);
        self.inner.promote_copy_stage(source, destination)?;
        if self.lost_publication.swap(false, Ordering::SeqCst) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "publication response lost",
            ));
        }
        Ok(())
    }
    fn rename(&self, source: &str, destination: &str) -> io::Result<()> {
        self.inner.rename(source, destination)
    }
    fn rename_no_replace(&self, source: &str, destination: &str) -> io::Result<()> {
        self.inner.rename_no_replace(source, destination)
    }
    fn remove_file(&self, path: &str) -> io::Result<()> {
        self.inner.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> io::Result<()> {
        self.inner.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> io::Result<()> {
        self.inner.mkdir_all(path)
    }
    fn create_dir_new(&self, path: &str) -> io::Result<()> {
        self.inner.create_dir_new(path)
    }
    fn discard_copy_stage(&self, path: &str) -> io::Result<()> {
        self.inner.discard_copy_stage(path)
    }
}
impl BackendExtensions for Limited {
    fn directory_reservation_supported(&self, _: &str) -> bool {
        true
    }
    fn create_dir_exclusive(&self, path: &str) -> io::Result<()> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        crate::vfs::create_dir_exclusive(&self.inner, path)?;
        if self.lost_directory.swap(false, Ordering::SeqCst) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "mkdir response lost",
            ));
        }
        Ok(())
    }
    fn directory_identity(&self, path: &str) -> io::Result<Option<String>> {
        if self.proofs {
            crate::vfs::directory_identity(&self.inner, path)
        } else {
            Ok(None)
        }
    }
    fn file_identity(&self, path: &str) -> io::Result<Option<String>> {
        if self.proofs {
            crate::vfs::file_identity(&self.inner, path)
        } else {
            Ok(None)
        }
    }
    fn rename_reserved_directory(&self, source: &str, destination: &str) -> io::Result<()> {
        self.reserved_renames.fetch_add(1, Ordering::SeqCst);
        if self.refuse_reserved {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "no atomic binding rename",
            ));
        }
        self.inner.rename_no_replace(source, destination)
    }
}

#[test]
fn batches_task_transfer_compat_folder_identity_does_not_require_binding_rename_for_first_copy() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("Photos");
    let target = root.path().join("target");
    write(&source.join("a.bin"), b"ours");
    fs::create_dir(&target).unwrap();
    let mut limited = Limited::new("id-without-binding");
    limited.proofs = true;
    limited.refuse_reserved = true;
    assert!(
        crate::vfs::directory_identity(&limited.inner, &fwd(&target))
            .unwrap()
            .is_some()
    );
    let backend = Arc::new(limited);
    let result = run(job(
        crate::transfer::Endpoint::Local,
        crate::transfer::Endpoint::Remote(backend.clone()),
        &target,
        &[&source],
    ));
    assert!(result.issues.is_empty(), "{:?}", result.issues);
    assert_eq!(backend.reserved_renames.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    assert_eq!(fs::read(target.join("Photos/a.bin")).unwrap(), b"ours");
}

#[derive(Clone)]
struct HttpRequest {
    method: String,
    path: String,
    headers: std::collections::HashMap<String, String>,
}
struct Dav {
    address: std::net::SocketAddr,
    requests: Arc<std::sync::Mutex<Vec<HttpRequest>>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Dav {
    fn new(resource_id: bool, rebind_status: u16) -> Self {
        use std::io::BufRead;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (quit, seen) = (stop.clone(), requests.clone());
        let worker = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !quit.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
                let (stream, _) = match listener.accept() {
                    Ok(stream) => stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                        continue;
                    }
                    Err(_) => return,
                };
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream);
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let mut words = line.split_whitespace();
                let method = words.next().unwrap_or("").to_owned();
                let path = words.next().unwrap_or("/").to_owned();
                let mut headers = std::collections::HashMap::new();
                loop {
                    line.clear();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
                    }
                }
                let length = headers
                    .get("content-length")
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(0usize);
                let mut body = vec![0; length];
                if reader.read_exact(&mut body).is_err() {
                    continue;
                }
                seen.lock().unwrap().push(HttpRequest {
                    method: method.clone(),
                    path: path.clone(),
                    headers,
                });
                let status = match method.as_str() {
                    "PROPFIND" => 207,
                    "MOVE" => 201,
                    "REBIND" => rebind_status,
                    _ => 405,
                };
                let properties = if resource_id {
                    "<d:resource-id><d:href>urn:uuid:own-directory</d:href></d:resource-id>"
                } else {
                    ""
                };
                let body = if status == 207 {
                    format!(
                        r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>{path}</d:href><d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype>{properties}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#
                    )
                } else {
                    String::new()
                };
                let response = format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = reader.get_mut().write_all(response.as_bytes());
            }
        });
        Self {
            address,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    fn backend(&self) -> crate::webdav::WebdavBackend {
        crate::webdav::WebdavBackend::connect(crate::webdav::WebdavConfig {
            https: false,
            host: self.address.ip().to_string(),
            port: self.address.port(),
            user: String::new(),
            password: String::new(),
            root: "/".into(),
        })
        .unwrap()
    }
}
impl Drop for Dav {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

#[test]
fn batches_task_transfer_compat_webdav_ordinary_folder_move_needs_no_binding_extension() {
    let server = Dav::new(false, 405);
    let backend = server.backend();
    backend.rename_no_replace("/old", "/new").unwrap();
    let requests = server.requests.lock().unwrap();
    let moved = requests
        .iter()
        .find(|request| request.method == "MOVE")
        .unwrap();
    assert_eq!(moved.path, "/old");
    assert_eq!(
        moved.headers.get("overwrite").map(String::as_str),
        Some("F")
    );
    assert!(moved.headers.get("destination").unwrap().ends_with("/new"));
    assert!(!requests.iter().any(|request| request.method == "REBIND"));
}

#[test]
fn batches_task_transfer_compat_webdav_reserved_move_keeps_strict_rebind_boundary_through_cache() {
    for (id, status, expected_rebind) in [(false, 405, 0), (true, 405, 1), (true, 201, 1)] {
        let server = Dav::new(id, status);
        let backend = crate::vfs::CachingBackend::new(Arc::new(server.backend()));
        let result = crate::vfs::rename_reserved_directory(&backend, "/owned", "/new");
        if id && status == 201 {
            assert!(result.is_ok());
        } else {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Unsupported);
        }
        let requests = server.requests.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.method == "REBIND")
                .count(),
            expected_rebind
        );
        assert!(!requests.iter().any(|request| request.method == "MOVE"));
    }
}

#[test]
fn batches_task_transfer_compat_folder_without_identity_copies_and_numbers() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("from/Photos");
    let target = root.path().join("target");
    write(&source.join("nested/a.bin"), b"ours");
    write(&target.join("Photos/foreign"), b"foreign");
    let backend = Arc::new(Limited::new("folder-no-id"));
    let outcome = run(job(
        crate::transfer::Endpoint::Local,
        crate::transfer::Endpoint::Remote(backend.clone()),
        &target,
        &[&source],
    ));
    assert!(outcome.issues.is_empty(), "{:?}", outcome.issues);
    assert_eq!(outcome.roots[0].rel, "Photos (2)");
    assert_eq!(
        fs::read(target.join("Photos (2)/nested/a.bin")).unwrap(),
        b"ours"
    );
    assert_eq!(fs::read(target.join("Photos/foreign")).unwrap(), b"foreign");
    assert_eq!(backend.creates.load(Ordering::SeqCst), 1);
}

#[test]
fn batches_task_transfer_compat_folder_lost_response_is_not_recovered_without_identity() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("Photos");
    let target = root.path().join("target");
    write(&source.join("a.bin"), b"ours");
    fs::create_dir(&target).unwrap();
    let backend = Arc::new(Limited::new("folder-lost-no-id"));
    backend.lost_directory.store(true, Ordering::SeqCst);
    let mut candidate = job(
        crate::transfer::Endpoint::Local,
        crate::transfer::Endpoint::Remote(backend.clone()),
        &target,
        &[&source],
    );
    let first = run(candidate.clone());
    assert!(!first.issues.is_empty());
    candidate.recovery = crate::transfer::RecoveryMode::Resume;
    let second = run(candidate);
    assert!(!second.issues.is_empty());
    assert_eq!(backend.creates.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read_dir(&target).unwrap().count(), 1);
    assert_eq!(fs::read_dir(target.join("Photos")).unwrap().count(), 0);
}

#[test]
fn batches_task_transfer_compat_missing_only_preserves_resolved_folder_and_skip_contract() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("from/Photos");
    let target = root.path().join("target");
    write(&source.join("a.bin"), b"aaa");
    write(&source.join("b.bin"), b"bbb");
    write(&source.join("c.bin"), b"ccc");
    write(&target.join("Photos/foreign"), b"foreign");
    let backend = Arc::new(Limited::new("missing-no-id"));
    let mut candidate = job(
        crate::transfer::Endpoint::Local,
        crate::transfer::Endpoint::Remote(backend.clone()),
        &target,
        &[&source],
    );
    let first = run(candidate.clone());
    assert!(first.issues.is_empty(), "{:?}", first.issues);
    fs::remove_file(target.join("Photos (2)/b.bin")).unwrap();
    write(&target.join("Photos (2)/c.bin"), b"different length");
    candidate.recovery = crate::transfer::RecoveryMode::Resume;
    candidate.resume = Some(first.roots);
    let previous = backend.promotes.load(Ordering::SeqCst);
    let second = run(candidate);
    assert_eq!(backend.promotes.load(Ordering::SeqCst) - previous, 1);
    assert_eq!(fs::read(target.join("Photos (2)/a.bin")).unwrap(), b"aaa");
    assert_eq!(fs::read(target.join("Photos (2)/b.bin")).unwrap(), b"bbb");
    assert_eq!(
        fs::read(target.join("Photos (2)/c.bin")).unwrap(),
        b"different length"
    );
    assert!(!target.join("Photos (3)").exists());
    assert_eq!(second.progress.errors, 1);
}

#[test]
fn batches_task_transfer_compat_missing_only_preserves_skipped_selected_folder() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    let folder = source.join("Photos");
    let single = source.join("a.bin");
    write(&folder.join("ours.bin"), b"ours");
    write(&single, b"aaa");
    write(&target.join("Photos/foreign"), b"foreign");
    let backend = Arc::new(Limited::new("missing-skip-folder"));
    let mut candidate = job(
        crate::transfer::Endpoint::Local,
        crate::transfer::Endpoint::Remote(backend),
        &target,
        &[&folder, &single],
    );
    candidate
        .folders
        .roots
        .insert(fwd(&folder), crate::transfer::FolderConflict::Skip);
    let first = run(candidate.clone());
    assert!(first.issues.is_empty(), "{:?}", first.issues);
    fs::remove_file(target.join("a.bin")).unwrap();
    candidate.recovery = crate::transfer::RecoveryMode::Resume;
    candidate.resume = Some(first.roots);
    let retry = run(candidate);
    assert!(retry.issues.is_empty(), "{:?}", retry.issues);
    assert_eq!(fs::read(target.join("a.bin")).unwrap(), b"aaa");
    assert!(!target.join("Photos/ours.bin").exists());
    assert_eq!(fs::read(target.join("Photos/foreign")).unwrap(), b"foreign");
}

#[test]
fn batches_task_transfer_compat_missing_only_remote_pairs_keep_literal_names_and_endpoints() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    write(&source.join("a#%.bin"), b"aaa");
    write(&source.join("b.bin"), b"bbb");
    fs::create_dir(&target).unwrap();
    let one = Arc::new(Limited::new("remote-source"));
    let two = Arc::new(Limited::new("remote-target"));
    let mut candidate = job(
        crate::transfer::Endpoint::Remote(one),
        crate::transfer::Endpoint::Remote(two.clone()),
        &target,
        &[],
    );
    candidate.items = JobItems::Pairs(vec![
        crate::transfer::PairItem::new(fwd(&source.join("a#%.bin")), "literal/a#%.bin"),
        crate::transfer::PairItem::new(fwd(&source.join("b.bin")), "literal/b.bin"),
    ]);
    let first = run(candidate.clone());
    assert!(first.issues.is_empty(), "{:?}", first.issues);
    fs::remove_file(target.join("literal/b.bin")).unwrap();
    candidate.recovery = crate::transfer::RecoveryMode::Resume;
    candidate.resume = Some(first.roots);
    let previous = two.promotes.load(Ordering::SeqCst);
    let second = run(candidate);
    assert!(second.issues.is_empty(), "{:?}", second.issues);
    assert_eq!(two.promotes.load(Ordering::SeqCst) - previous, 1);
    assert_eq!(fs::read(target.join("literal/a#%.bin")).unwrap(), b"aaa");
    assert_eq!(fs::read(target.join("literal/b.bin")).unwrap(), b"bbb");
}
