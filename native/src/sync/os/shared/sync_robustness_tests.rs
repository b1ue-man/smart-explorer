//! The mirror's copy pass under pressure: overload is waited out instead of
//! reported, a budget stop ends the pass, a link swapped into a local
//! destination is refused, a panicking worker ends the pass, and a stale
//! directory entry is confirmed by a `stat`.
use super::*;
use crate::bisync::link_fixture;
use crate::bisync::test_remote::{FakeRemote, REMOTE_ROOT};
use crate::vfs::{BackendHandle, LocalBackend, Scheme, VfsMeta, VfsResult};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn forward(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn write(root: &Path, rel: &str, content: &[u8]) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn mirror(
    source: BackendHandle,
    source_root: &str,
    destination: BackendHandle,
    destination_root: &str,
    delete_extra: bool,
    dry_run: bool,
) -> SyncResult {
    let (tx, rx) = crossbeam_channel::unbounded();
    let _handle = start_sync(
        source,
        source_root.to_string(),
        destination,
        destination_root.to_string(),
        SyncOptions {
            delete_extra,
            dry_run,
        },
        tx,
    );
    loop {
        match rx.recv_timeout(Duration::from_secs(60)) {
            Ok(SyncMsg::Done(result)) => return result,
            Ok(SyncMsg::Progress(_)) => {}
            Err(error) => panic!("sync did not finish: {error}"),
        }
    }
}

fn files_in(path: &Path) -> usize {
    std::fs::read_dir(path)
        .map(|entries| entries.count())
        .unwrap_or(0)
}

#[test]
fn transfer_engine_task_sync_overload_is_waited_out_not_reported() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    for index in 0..12 {
        write(
            a.path(),
            &format!("dir{}/f{index}.txt", index % 3),
            b"payload",
        );
    }
    let source = Arc::new(
        FakeRemote::new(a.path(), "busy-source")
            .with_overload("list", 2)
            .with_overload("read", 3),
    );
    let destination = Arc::new(FakeRemote::new(b.path(), "busy-target").with_overload("write", 3));
    let source_handle: BackendHandle = source.clone();
    let destination_handle: BackendHandle = destination.clone();

    let result = mirror(
        source_handle,
        REMOTE_ROOT,
        destination_handle,
        REMOTE_ROOT,
        true,
        false,
    );

    assert!(result.errors.is_empty(), "{:?}", result.errors);
    assert_eq!(result.stats.copied, 12);
    assert_eq!(source.calls.congested.load(Ordering::SeqCst), 5);
    assert_eq!(destination.calls.congested.load(Ordering::SeqCst), 3);
    for index in 0..12 {
        let rel = format!("dir{}/f{index}.txt", index % 3);
        assert_eq!(std::fs::read(b.path().join(rel)).unwrap(), b"payload");
    }
}

/// Endless nesting: `/deep/x/d/d/…` and `/deep/y/d/d/…`, listed at once.
struct EndlessTree {
    identity: String,
}

impl crate::vfs::Backend for EndlessTree {
    fn scheme(&self) -> Scheme {
        Scheme::Sftp
    }
    fn root_display(&self) -> String {
        "/deep".to_string()
    }
    fn state_identity(&self) -> String {
        self.identity.clone()
    }
    fn flow_key(&self, _path: &str) -> String {
        self.identity.clone()
    }
    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let names: &[&str] = if path == "/deep" { &["x", "y"] } else { &["d"] };
        Ok(names
            .iter()
            .map(|name| VfsMeta {
                name: name.to_string(),
                is_dir: true,
                ..Default::default()
            })
            .collect())
    }
    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        Ok(VfsMeta {
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            is_dir: true,
            ..Default::default()
        })
    }
    fn open_read(&self, _path: &str) -> VfsResult<Box<dyn Read + Send>> {
        Err(std::io::Error::other("no files here"))
    }
    fn open_write(&self, _path: &str) -> VfsResult<Box<dyn Write + Send>> {
        Err(std::io::Error::other("read-only"))
    }
    fn rename(&self, _source: &str, _destination: &str) -> VfsResult<()> {
        Err(std::io::Error::other("read-only"))
    }
    fn remove_file(&self, _path: &str) -> VfsResult<()> {
        Err(std::io::Error::other("read-only"))
    }
    fn remove_dir(&self, _path: &str) -> VfsResult<()> {
        Err(std::io::Error::other("read-only"))
    }
    fn mkdir_all(&self, _path: &str) -> VfsResult<()> {
        Err(std::io::Error::other("read-only"))
    }
}

#[test]
fn transfer_engine_task_sync_budget_stop_ends_the_pass() {
    let holder = tempfile::tempdir().unwrap();
    let missing = holder.path().join("not-created");
    let source: BackendHandle = Arc::new(EndlessTree {
        identity: format!("endless-tree:{}", std::process::id()),
    });
    let destination: BackendHandle = Arc::new(LocalBackend::new(&forward(holder.path())));

    // A dry run into a missing folder: only the source is walked, deeper
    // and deeper on two branches at once, until the depth budget stops it.
    let result = mirror(
        source,
        "/deep",
        destination,
        &forward(&missing),
        false,
        true,
    );

    assert!(
        result
            .errors
            .iter()
            .any(|(_, message)| message.contains("exceeds 512 levels")),
        "{:?}",
        result.errors
    );
    assert!(!missing.exists());
}

/// Replaces the destination folder `folder` (empty) by a link to `target`
/// the first time the source is asked for `operation` on `path`.
fn swap_on(
    operation: &'static str,
    path: String,
    folder: PathBuf,
    target: PathBuf,
) -> crate::bisync::test_remote::Hook {
    let swapped = AtomicBool::new(false);
    Arc::new(move |seen: &str, seen_path: &str| {
        if seen == operation && seen_path == path && !swapped.swap(true, Ordering::SeqCst) {
            std::fs::remove_dir(&folder).unwrap();
            link_fixture::directory(&target, &folder);
        }
    })
}

#[test]
fn transfer_engine_task_sync_listed_folder_swapped_for_link_is_refused() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let victim = tempfile::tempdir().unwrap();
    write(a.path(), "sub/file.txt", b"must not leave the destination");
    std::fs::create_dir(b.path().join("sub")).unwrap();
    // Swapped after the root listing saw `sub` as a plain folder, while the
    // source side of `sub` is listed.
    let hook = swap_on(
        "list",
        format!("{REMOTE_ROOT}/sub"),
        b.path().join("sub"),
        victim.path().to_path_buf(),
    );
    let source: BackendHandle = Arc::new(FakeRemote::new(a.path(), "swap-listed").with_hook(hook));
    let destination: BackendHandle = Arc::new(LocalBackend::new(&forward(b.path())));

    let result = mirror(
        source,
        REMOTE_ROOT,
        destination,
        &forward(b.path()),
        true,
        false,
    );

    assert!(!result.errors.is_empty());
    assert_eq!(result.stats.deleted, 0);
    assert_eq!(files_in(victim.path()), 0);
    link_fixture::remove_directory(&b.path().join("sub"));
}

#[test]
fn transfer_engine_task_sync_local_parent_swapped_for_link_blocks_the_copy() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let victim = tempfile::tempdir().unwrap();
    write(a.path(), "new/only.txt", b"must not leave the destination");
    // `new` is created by the pass; it becomes a link while the file is read.
    let hook = swap_on(
        "read",
        format!("{REMOTE_ROOT}/new/only.txt"),
        b.path().join("new"),
        victim.path().to_path_buf(),
    );
    let source: BackendHandle = Arc::new(FakeRemote::new(a.path(), "swap-parent").with_hook(hook));
    let destination: BackendHandle = Arc::new(LocalBackend::new(&forward(b.path())));

    let result = mirror(
        source,
        REMOTE_ROOT,
        destination,
        &forward(b.path()),
        false,
        false,
    );

    assert!(result
        .errors
        .iter()
        .any(|(path, _)| path.ends_with("new/only.txt")));
    assert_eq!(result.stats.copied, 0);
    assert_eq!(files_in(victim.path()), 0);
    link_fixture::remove_directory(&b.path().join("new"));
}

#[test]
fn transfer_engine_task_sync_panicking_worker_ends_the_pass() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    for index in 0..6 {
        write(a.path(), &format!("f{index}.txt"), b"data");
    }
    write(a.path(), "boom.txt", b"data");
    write(b.path(), "orphan.txt", b"stays");
    let reads = Arc::new(AtomicUsize::new(0));
    let counted = reads.clone();
    let source = FakeRemote::new(a.path(), "panicking").with_hook(Arc::new(
        move |operation: &str, path: &str| {
            if operation == "read" {
                counted.fetch_add(1, Ordering::SeqCst);
                if path.ends_with("boom.txt") {
                    panic!("injected worker panic");
                }
            }
        },
    ));
    let source: BackendHandle = Arc::new(source);
    let destination: BackendHandle = Arc::new(LocalBackend::new(&forward(b.path())));

    let result = mirror(
        source,
        REMOTE_ROOT,
        destination,
        &forward(b.path()),
        true,
        false,
    );

    assert!(result
        .errors
        .iter()
        .any(|(_, message)| message.contains("stopped unexpectedly")));
    assert_eq!(result.stats.deleted, 0);
    assert!(b.path().join("orphan.txt").exists());
    assert!(!b.path().join("boom.txt").exists());
    assert!(reads.load(Ordering::SeqCst) >= 1);
}

#[test]
fn transfer_engine_task_sync_stale_listing_is_confirmed_by_stat() {
    let a = tempfile::tempdir().unwrap();
    let smb = tempfile::tempdir().unwrap();
    let ftp = tempfile::tempdir().unwrap();
    write(a.path(), "same.txt", b"identical");
    for target in [smb.path(), ftp.path()] {
        write(target, "same.txt", b"identical");
    }
    let run = |target: &Path, scheme: Scheme| {
        // The directory entry shows a stale size; `stat` shows the truth.
        let destination: BackendHandle = Arc::new(
            FakeRemote::new(target, "stale")
                .with_scheme(scheme)
                .with_stale_listing("same.txt", 1),
        );
        let source: BackendHandle = Arc::new(LocalBackend::new(&forward(a.path())));
        mirror(
            source,
            &forward(a.path()),
            destination,
            REMOTE_ROOT,
            false,
            false,
        )
    };

    // Cheap `stat` (SMB, local): the decision follows the real file.
    let confirmed = run(smb.path(), Scheme::Smb);
    assert!(confirmed.errors.is_empty(), "{:?}", confirmed.errors);
    assert_eq!(confirmed.stats.skipped, 1);
    assert_eq!(confirmed.stats.copied, 0);
    // FTP relies on its listing: the stale entry leads to a copy attempt,
    // which the final check before publishing refuses (nothing replaced).
    let listed = run(ftp.path(), Scheme::Ftp);
    assert_eq!(listed.stats.errors, 1);
    assert_eq!(
        std::fs::read(ftp.path().join("same.txt")).unwrap(),
        b"identical"
    );
}
