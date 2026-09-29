use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct Source {
    scheme: Scheme,
    opens: AtomicUsize,
    failures: usize,
    kind: io::ErrorKind,
    fail_open: bool,
    bytes: Vec<u8>,
    metadata_size: u64,
    export: bool,
    drift: bool,
    seen_ids: Mutex<Vec<Option<String>>>,
}

impl Default for Source {
    fn default() -> Self {
        Self { scheme: Scheme::Peer, opens: AtomicUsize::new(0), failures: 0,
            kind: io::ErrorKind::NotConnected, fail_open: false,
            bytes: vec![b'B'; 1024], metadata_size: 1024, export: false,
            drift: false, seen_ids: Mutex::new(Vec::new()) }
    }
}

struct BrokenReader(bool, io::ErrorKind);

impl Read for BrokenReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.0 {
            return Err(io::Error::new(self.1, "injected remote read failure"));
        }
        self.0 = true;
        let n = out.len().min(17);
        out[..n].fill(b'A');
        Ok(n)
    }
}

impl Backend for Source {
    fn scheme(&self) -> Scheme { self.scheme }
    fn root_display(&self) -> String { "/".into() }
    fn list_dir(&self, _: &str) -> io::Result<Vec<VfsMeta>> { Ok(vec![self.stat("/file")?]) }
    fn stat(&self, _: &str) -> io::Result<VfsMeta> {
        Ok(VfsMeta { name: "file".into(), size: self.metadata_size,
            mtime_ms: if self.drift && self.opens.load(Ordering::SeqCst) > 0 { 8 } else { 7 },
            id: Some("path-item".into()), ..Default::default() })
    }
    fn read_size(&self, _: &str, size: u64) -> io::Result<Option<u64>> {
        Ok((!self.export).then_some(size))
    }
    fn open_read(&self, path: &str) -> io::Result<Box<dyn Read + Send>> {
        self.open_read_id(path, None)
    }
    fn open_read_id(&self, _: &str, id: Option<&str>) -> io::Result<Box<dyn Read + Send>> {
        self.seen_ids.lock().unwrap().push(id.map(str::to_string));
        let attempt = self.opens.fetch_add(1, Ordering::SeqCst);
        if attempt < self.failures {
            if self.fail_open {
                return Err(io::Error::new(self.kind, "injected remote open failure"));
            }
            return Ok(Box::new(BrokenReader(false, self.kind)));
        }
        Ok(Box::new(io::Cursor::new(self.bytes.clone())))
    }
    fn open_write(&self, _: &str) -> io::Result<Box<dyn Write + Send>> { panic!("download mutated remote") }
    fn rename(&self, _: &str, _: &str) -> io::Result<()> { panic!("download renamed remote") }
    fn remove_file(&self, _: &str) -> io::Result<()> { panic!("download deleted remote") }
    fn remove_dir(&self, _: &str) -> io::Result<()> { panic!("download deleted remote") }
    fn mkdir_all(&self, _: &str) -> io::Result<()> { panic!("download created remote directory") }
}

fn assert_no_parts(directory: &Path, destination_exists: bool) {
    let paths: Vec<_> = std::fs::read_dir(directory).unwrap().map(|e| e.unwrap().path()).collect();
    assert_eq!(paths.len(), usize::from(destination_exists), "leftover download stage: {paths:?}");
}

#[test]
fn direct_open_task_stream_retry_restarts_bytes_through_real_daemon_ipc() {
    let source = Arc::new(Source { failures: 1, ..Default::default() });
    let bridge = crate::daemon::DirectOpenTaskBridge::new(source.clone()).unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let destination = temporary.path().join("photo Ü #.jpg");
    let (_, revision) = download_for_edit(&*bridge.backend, "/A/photo Ü #.jpg", None, &destination).unwrap();
    assert_eq!(revision, 7);
    assert_eq!(source.opens.load(Ordering::SeqCst), 2);
    assert_eq!(std::fs::read(&destination).unwrap(), source.bytes);
    assert_no_parts(temporary.path(), true);
}

#[test]
fn direct_open_task_open_failure_retries_but_terminal_failures_and_other_backends_do_not() {
    for (scheme, kind, expected_opens) in [
        (Scheme::Peer, io::ErrorKind::TimedOut, 2),
        (Scheme::Peer, io::ErrorKind::PermissionDenied, 1),
        (Scheme::Peer, io::ErrorKind::InvalidData, 1),
        (Scheme::Sftp, io::ErrorKind::NotConnected, 1),
        (Scheme::Ftp, io::ErrorKind::NotConnected, 1),
        (Scheme::Webdav, io::ErrorKind::NotConnected, 1),
        (Scheme::GDrive, io::ErrorKind::NotConnected, 1),
    ] {
        let source = Source { scheme, kind, failures: 1, fail_open: true, ..Default::default() };
        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary.path().join("file");
        let result = download_for_edit(&source, "/file", None, &destination);
        assert_eq!(result.is_ok(), expected_opens == 2, "{scheme:?} {kind:?}");
        assert_eq!(source.opens.load(Ordering::SeqCst), expected_opens);
        assert_no_parts(temporary.path(), result.is_ok());
    }
}

#[test]
fn direct_open_task_exhausted_retry_keeps_previous_destination_and_cleans_stages() {
    let source = Source { failures: usize::MAX, ..Default::default() };
    let temporary = tempfile::tempdir().unwrap();
    let destination = temporary.path().join("file");
    std::fs::write(&destination, b"previous complete copy").unwrap();
    assert!(download_for_edit(&source, "/file", None, &destination).is_err());
    assert_eq!(source.opens.load(Ordering::SeqCst), MAX_ATTEMPTS);
    assert_eq!(std::fs::read(&destination).unwrap(), b"previous complete copy");
    assert_no_parts(temporary.path(), true);
}

#[test]
fn direct_open_task_changed_truncated_and_grown_source_never_publish() {
    for source in [
        Source { drift: true, ..Default::default() },
        Source { bytes: vec![b'B'; 10], ..Default::default() },
        Source { bytes: vec![b'B'; 1025], ..Default::default() },
        Source { metadata_size: 0, ..Default::default() },
    ] {
        let source = Arc::new(source);
        let cached = crate::vfs::CachingBackend::new(source.clone());
        cached.list_dir("/").unwrap();
        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary.path().join("file");
        assert!(download_for_edit(&cached, "/file", None, &destination).is_err());
        assert_eq!(source.opens.load(Ordering::SeqCst), 1);
        assert_no_parts(temporary.path(), false);
    }
}

#[test]
fn direct_open_task_empty_exported_and_id_selected_files_keep_their_contract() {
    for (source, id, revision) in [
        (Source { bytes: Vec::new(), metadata_size: 0, ..Default::default() }, None, 7),
        (Source { scheme: Scheme::GDrive, metadata_size: 0, export: true,
            ..Default::default() }, None, 7),
        (Source { scheme: Scheme::GDrive, metadata_size: 10,
            ..Default::default() }, Some("different-selected-item"), 0),
        (Source { metadata_size: 10, ..Default::default() },
            Some("different-selected-item"), 0),
    ] {
        let temporary = tempfile::tempdir().unwrap();
        let destination = temporary.path().join("file");
        let (_, actual_revision) = download_for_edit(&source, "/duplicate name", id, &destination).unwrap();
        assert_eq!(actual_revision, revision);
        assert_eq!(std::fs::read(&destination).unwrap(), source.bytes);
        assert_eq!(*source.seen_ids.lock().unwrap(), vec![id.map(str::to_string)]);
        assert_no_parts(temporary.path(), true);
    }
}

#[test]
fn direct_open_task_local_write_failure_is_never_a_remote_retry() {
    struct FailingDisk;
    impl Write for FailingDisk {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "local destination unavailable"))
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    let error = copy_checked(&mut io::Cursor::new(b"file"), &mut FailingDisk, Some(4))
        .err().expect("local write failure");
    assert!(!error.retryable());
    assert!(error.message().contains("local destination"));
}
