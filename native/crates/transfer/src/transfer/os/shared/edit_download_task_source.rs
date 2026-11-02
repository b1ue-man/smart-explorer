//! The remote double of the edit-download tests: a source that fails, drifts,
//! exports or truncates on request. The transfer tests and the daemon's
//! direct-open test share it (plan docs/plaene/2026-11-01-idealzustand, D13).
use crate::vfs::{Backend, Scheme, VfsMeta};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

pub struct Source {
    pub scheme: Scheme,
    pub opens: AtomicUsize,
    pub failures: usize,
    pub kind: io::ErrorKind,
    pub fail_open: bool,
    pub bytes: Vec<u8>,
    pub metadata_size: u64,
    pub export: bool,
    pub drift: bool,
    pub seen_ids: Mutex<Vec<Option<String>>>,
}

impl Default for Source {
    fn default() -> Self {
        Self {
            scheme: Scheme::Peer,
            opens: AtomicUsize::new(0),
            failures: 0,
            kind: io::ErrorKind::NotConnected,
            fail_open: false,
            bytes: vec![b'B'; 1024],
            metadata_size: 1024,
            export: false,
            drift: false,
            seen_ids: Mutex::new(Vec::new()),
        }
    }
}

impl Backend for Source {
    fn scheme(&self) -> Scheme {
        self.scheme
    }
    fn root_display(&self) -> String {
        "/".into()
    }
    fn list_dir(&self, _: &str) -> io::Result<Vec<VfsMeta>> {
        Ok(vec![self.stat("/file")?])
    }
    fn stat(&self, _: &str) -> io::Result<VfsMeta> {
        Ok(VfsMeta {
            name: "file".into(),
            size: self.metadata_size,
            mtime_ms: if self.drift && self.opens.load(Ordering::SeqCst) > 0 {
                8
            } else {
                7
            },
            id: Some("path-item".into()),
            ..Default::default()
        })
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
    fn open_write(&self, _: &str) -> io::Result<Box<dyn Write + Send>> {
        panic!("download mutated remote")
    }
    fn rename(&self, _: &str, _: &str) -> io::Result<()> {
        panic!("download renamed remote")
    }
    fn remove_file(&self, _: &str) -> io::Result<()> {
        panic!("download deleted remote")
    }
    fn remove_dir(&self, _: &str) -> io::Result<()> {
        panic!("download deleted remote")
    }
    fn mkdir_all(&self, _: &str) -> io::Result<()> {
        panic!("download created remote directory")
    }
}

pub fn assert_no_parts(directory: &Path, destination_exists: bool) {
    let paths: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(
        paths.len(),
        usize::from(destination_exists),
        "leftover download stage: {paths:?}"
    );
}
