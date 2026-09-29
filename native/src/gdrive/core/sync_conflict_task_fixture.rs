use super::task_drive::{drive_error, FakeDrive};
use super::task_http::Server;
use super::GDriveBackend;
use crate::bisync::{self, BisyncOptions, Conflict, ResolvePhase, WalkFilter};
use crate::vfs::{Backend, LocalBackend};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub(super) const FILE: &str = "appearance.json";
pub(super) const MIME: &str = "application/octet-stream";

#[derive(Default)]
pub(super) struct Faults {
    pub deny_read: AtomicBool,
    pub deny_trash: AtomicBool,
    pub deny_replace: AtomicBool,
    pub lose_trash_ack: AtomicBool,
}

pub(super) struct Fixture {
    pub drive: Arc<FakeDrive>,
    pub server: Server,
    pub faults: Arc<Faults>,
    pub local: LocalBackend,
    pub root: String,
    pub remote: GDriveBackend,
    pub pair: String,
    pub directory: tempfile::TempDir,
}

impl Fixture {
    pub fn new(local: Option<&[u8]>, files: &[(&str, &[u8])]) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_string_lossy().replace('\\', "/");
        if let Some(bytes) = local { std::fs::write(directory.path().join(FILE), bytes).unwrap(); }
        let drive = Arc::new(FakeDrive::default());
        for (id, bytes) in files { drive.insert(id, FILE, "root", MIME, bytes); }
        let faults = Arc::new(Faults::default());
        let server = {
            let (drive, faults) = (drive.clone(), faults.clone());
            Server::start(move |request| {
                if request.query("alt").as_deref() == Some("media") && faults.deny_read.load(Ordering::Acquire) {
                    return drive_error(403, "insufficientFilePermissions", "reading denied");
                }
                if request.method == "PATCH" && request.path().starts_with("/upload/")
                    && faults.deny_replace.load(Ordering::Acquire) {
                    return drive_error(403, "insufficientFilePermissions", "replacement denied");
                }
                let trash = request.method == "PATCH" && request.body.windows(7).any(|b| b == b"trashed");
                if trash && faults.deny_trash.load(Ordering::Acquire) {
                    return drive_error(403, "insufficientFilePermissions", "trash denied");
                }
                let answer = drive.answer(request);
                if trash && faults.lose_trash_ack.swap(false, Ordering::AcqRel) {
                    return drive_error(503, "backendError", "acknowledgement lost");
                }
                answer
            })
        };
        let remote = server.backend();
        let local = LocalBackend::new(&root);
        let pair = bisync::pair_id_for(&local, &root, &remote, "/");
        Self { drive, server, faults, local, root, remote, pair, directory }
    }

    pub fn preview(&self) -> bisync::Preview {
        self.preview_with(BisyncOptions::default(), &filter(&bisync::empty_globset()))
    }

    pub fn preview_with(&self, opts: BisyncOptions, filter: &WalkFilter<'_>) -> bisync::Preview {
        bisync::preview(&self.local, &self.root, &self.remote, "/", opts, &AtomicBool::new(false), filter)
    }

    pub fn run(&self, opts: BisyncOptions) -> bisync::Outcome {
        bisync::run(&self.local, &self.root, &self.remote, "/", opts, &AtomicBool::new(false),
            &filter(&bisync::empty_globset()))
    }

    pub fn conflict(&self) -> Conflict {
        let mut preview = self.preview();
        assert!(preview.error.is_none(), "{:?}", preview.error);
        assert_eq!(preview.conflicts.len(), 1);
        preview.conflicts.remove(0)
    }

    pub fn resolve(&self, conflict: &Conflict, keep_a: bool, id: Option<&str>, cancel: &AtomicBool,
        progress: impl FnMut(ResolvePhase)) -> io::Result<(Option<bisync::Sig>, Option<bisync::Sig>)> {
        bisync::resolve_variant_checked(&self.local, &self.root, &self.remote, "/", conflict,
            keep_a, id, &self.pair, cancel, progress)
    }

    pub fn remote_content(&self) -> Vec<u8> {
        let files = self.drive.named("root", FILE);
        assert_eq!(files.len(), 1, "exactly one live remote file");
        let mut content = Vec::new();
        self.remote.open_read_id(FILE, files[0]["id"].as_str()).unwrap().read_to_end(&mut content).unwrap();
        content
    }

    pub fn local_content(&self) -> Vec<u8> { std::fs::read(self.directory.path().join(FILE)).unwrap() }

    pub fn assert_backed_up(&self, bytes: &[u8]) {
        assert!(contains(&bisync::versions_dir(&self.pair), bytes), "discarded bytes have a durable backup");
    }

    pub fn mutation_count(&self) -> usize {
        self.server.requests().iter().filter(|r| r.method != "GET").count()
    }
}

pub(super) fn filter(ignore: &globset::GlobSet) -> WalkFilter<'_> {
    WalkFilter { include_hidden: true, ignore, min_size: 0, max_size: 0,
        after_mtime_ms: 0, before_mtime_ms: 0 }
}

fn contains(root: &Path, expected: &[u8]) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else { return false; };
    entries.flatten().any(|entry| {
        let path = entry.path();
        if path.is_dir() { contains(&path, expected) }
        else { std::fs::read(path).ok().as_deref() == Some(expected) }
    })
}
