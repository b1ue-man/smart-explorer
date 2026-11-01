//! The checksum compare mode against a remote side that forms MD5s for many
//! files at once (`vfs::content_md5_many`, SFTP without an agent): the walk
//! asks once per folder with all of its files, never file by file, and moves
//! no content; a file the side leaves unanswered goes the single-file way.
//! Unix only (absolute `/` paths of the local stand-in).
#![cfg(unix)]
use super::super::snapshot::{empty_globset, walk_files, WalkFilter};
use super::super::snapshot_hash::{md5_to_u64, HashMode};
use crate::vfs::{Backend, BackendExtensions, LocalBackend, VfsMeta, VfsResult};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

/// A remote side over local files that hashes where the files are, like the
/// SFTP exec way: every batched call and every download is counted.
struct HashingRemote {
    local: LocalBackend,
    batches: Mutex<Vec<usize>>,
    singles: AtomicUsize,
    downloads: AtomicUsize,
    /// Paths the batched call leaves unanswered (`None`).
    unanswered: Vec<String>,
}

impl HashingRemote {
    fn new(unanswered: Vec<String>) -> Self {
        Self {
            local: LocalBackend::new("/"),
            batches: Mutex::new(Vec::new()),
            singles: AtomicUsize::new(0),
            downloads: AtomicUsize::new(0),
            unanswered,
        }
    }
}

fn md5_hex_of(path: &str) -> String {
    format!("{:x}", md5::compute(std::fs::read(path).unwrap()))
}

impl Backend for HashingRemote {
    fn scheme(&self) -> crate::vfs::Scheme {
        self.local.scheme()
    }
    fn root_display(&self) -> String {
        self.local.root_display()
    }
    fn is_local(&self) -> bool {
        false
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        self.local.list_dir(path)
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        self.local.stat(path)
    }
    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        if path.ends_with(".txt") {
            self.downloads.fetch_add(1, Ordering::Relaxed);
        }
        self.local.open_read(path)
    }
    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.local.open_write(path)
    }
    fn rename(&self, source: &str, destination: &str) -> VfsResult<()> {
        self.local.rename(source, destination)
    }
    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.local.remove_file(path)
    }
    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.local.remove_dir(path)
    }
    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        self.local.mkdir_all(path)
    }
    fn extensions(&self) -> Option<&dyn BackendExtensions> {
        Some(self)
    }
}

impl BackendExtensions for HashingRemote {
    fn content_md5(&self, path: &str, _cancel: &AtomicBool) -> VfsResult<Option<String>> {
        self.singles.fetch_add(1, Ordering::Relaxed);
        Ok(Some(md5_hex_of(path)))
    }

    fn content_md5_many(
        &self,
        files: &[(&str, &VfsMeta)],
        _cancel: &AtomicBool,
    ) -> VfsResult<Option<Vec<Option<String>>>> {
        self.batches.lock().unwrap().push(files.len());
        Ok(Some(
            files
                .iter()
                .map(|(path, listed)| {
                    assert!(!listed.is_dir, "only files are asked: {path}");
                    (!self.unanswered.iter().any(|skip| skip == path)).then(|| md5_hex_of(path))
                })
                .collect(),
        ))
    }
}

/// `folders` folders of `per_folder` files each below a temporary root.
fn tree(folders: usize, per_folder: usize) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    for folder in 0..folders {
        let path = dir.path().join(format!("d{folder}"));
        std::fs::create_dir_all(&path).unwrap();
        for file in 0..per_folder {
            std::fs::write(
                path.join(format!("f{file}.txt")),
                format!("{folder}/{file}"),
            )
            .unwrap();
        }
    }
    let root = dir.path().to_str().unwrap().to_string();
    (dir, root)
}

#[test]
fn abgleich_task_checksum_walk_asks_one_batch_per_folder_and_loads_no_file() {
    let (_dir, root) = tree(12, 25);
    let remote = HashingRemote::new(Vec::new());
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let cancel = AtomicBool::new(false);
    let found = walk_files(&remote, &root, &cancel, &filter, HashMode::FullFresh, None).unwrap();
    assert_eq!(found.len(), 12 * 25);
    for folder in 0..12 {
        for file in 0..25 {
            let sig = &found[&format!("d{folder}/f{file}.txt")];
            assert_eq!(
                sig.hash,
                md5_to_u64(&md5::compute(format!("{folder}/{file}")).0),
                "d{folder}/f{file}.txt"
            );
        }
    }
    let mut batches = remote.batches.lock().unwrap().clone();
    batches.sort_unstable();
    assert_eq!(
        batches,
        vec![25; 12],
        "one batched call per folder with all its files"
    );
    assert_eq!(
        remote.singles.load(Ordering::Relaxed),
        0,
        "no file asked alone"
    );
    assert_eq!(
        remote.downloads.load(Ordering::Relaxed),
        0,
        "no file content moved"
    );
}

#[test]
fn abgleich_task_checksum_walk_asks_an_unanswered_file_on_its_own() {
    let (_dir, root) = tree(1, 4);
    let skipped = format!("{root}/d0/f2.txt");
    let remote = HashingRemote::new(vec![skipped]);
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let cancel = AtomicBool::new(false);
    let found = walk_files(&remote, &root, &cancel, &filter, HashMode::FullFresh, None).unwrap();
    assert_eq!(found.len(), 4);
    assert_eq!(
        found["d0/f2.txt"].hash,
        md5_to_u64(&md5::compute("0/2").0),
        "the single-file way answered it"
    );
    assert_eq!(*remote.batches.lock().unwrap(), vec![4]);
    assert_eq!(remote.singles.load(Ordering::Relaxed), 1);
    assert_eq!(remote.downloads.load(Ordering::Relaxed), 0);
}

#[test]
fn abgleich_task_mtime_walk_without_content_hashing_asks_nothing() {
    let (_dir, root) = tree(2, 3);
    let remote = HashingRemote::new(Vec::new());
    let globs = empty_globset();
    let filter = WalkFilter::basic(true, &globs);
    let cancel = AtomicBool::new(false);
    let found = walk_files(&remote, &root, &cancel, &filter, HashMode::None, None).unwrap();
    assert_eq!(found.len(), 6);
    assert!(found.values().all(|sig| sig.hash == 0));
    assert!(remote.batches.lock().unwrap().is_empty());
    assert_eq!(remote.singles.load(Ordering::Relaxed), 0);
}
