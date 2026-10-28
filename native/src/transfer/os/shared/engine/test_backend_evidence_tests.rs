//! Requests per file against a fake Share host (K-B, K-F): with transfer
//! evidence a packet member costs no `stat`, no `file_identity` and no
//! folder creation of its own; without it the engine keeps today's sequence
//! (a presence stat before, a stat and an identity after each member).
use super::{fwd, job, run, write, Fake};
use crate::transfer::Endpoint;
use crate::vfs::BatchLimits;
use std::path::Path;
use std::sync::atomic::Ordering;

const FOLDERS: usize = 3;

/// Requests of one job: `stat`, `file_identity`, single folder creations,
/// bulk folder creations, upload packets.
#[derive(Debug, PartialEq, Eq)]
struct Requests {
    stats: usize,
    identities: usize,
    dirs: usize,
    bulk_dirs: usize,
    packets: usize,
}

fn requests(fake: &Fake) -> Requests {
    let counters = &fake.counters;
    Requests {
        stats: counters.stats.load(Ordering::SeqCst),
        identities: counters.identities.load(Ordering::SeqCst),
        dirs: counters.dirs.load(Ordering::SeqCst),
        bulk_dirs: counters.bulk_dirs.load(Ordering::SeqCst),
        packets: counters.put_batches.load(Ordering::SeqCst)
            + counters.get_batches.load(Ordering::SeqCst),
    }
}

fn fake(tag: &str, evidence: bool) -> Fake {
    let mut fake = Fake::new(tag);
    fake.identity_proofs = true;
    fake.evidence = evidence;
    fake.bulk_dirs = evidence;
    fake.batch = Some(BatchLimits {
        max_files: 256,
        max_bytes: 16 * 1024 * 1024,
    });
    fake
}

fn content(index: usize) -> Vec<u8> {
    format!("small file {index:04}").into_bytes()
}

fn tree(root: &Path, files: usize) {
    for index in 0..files {
        let folder = format!("d{}", index % FOLDERS);
        write(
            &root.join(folder).join(format!("f{index:04}.txt")),
            &content(index),
        );
    }
}

fn check_tree(root: &Path, files: usize) {
    for index in 0..files {
        let folder = format!("d{}", index % FOLDERS);
        let path = root.join(folder).join(format!("f{index:04}.txt"));
        assert_eq!(std::fs::read(&path).expect("copied file"), content(index));
    }
}

fn upload(files: usize, evidence: bool) -> Requests {
    let root = tempfile::tempdir().expect("temp dir");
    let local = root.path().join("local/Vault");
    tree(&local, files);
    let remote = root.path().join("remote");
    std::fs::create_dir(&remote).expect("remote");
    let (fake, handle) = fake("evidence-upload", evidence).handle();
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &remote,
        &[&local],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert!(finished.errors.is_empty(), "{:?}", finished.errors);
    assert_eq!(finished.progress.files_done, files as u64);
    check_tree(&remote.join("Vault"), files);
    requests(&fake)
}

fn download(files: usize, evidence: bool) -> Requests {
    let root = tempfile::tempdir().expect("temp dir");
    let remote = root.path().join("remote/Vault");
    tree(&remote, files);
    let dest = root.path().join("dest");
    std::fs::create_dir(&dest).expect("dest");
    let mut source = fake("evidence-download", evidence);
    // Listed MD5s prove each read, so no file needs a check stat after it.
    for index in 0..files {
        let folder = format!("d{}", index % FOLDERS);
        let path = remote.join(folder).join(format!("f{index:04}.txt"));
        let digest = format!("{:x}", md5::compute(content(index)));
        source.md5.insert(fwd(&path), digest);
    }
    let (source, handle) = source.handle();
    let finished = run(job(
        Endpoint::Remote(handle),
        Endpoint::Local,
        &dest,
        &[&remote],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert!(finished.errors.is_empty(), "{:?}", finished.errors);
    assert_eq!(finished.progress.files_done, files as u64);
    check_tree(&dest.join("Vault"), files);
    requests(&source)
}

#[test]
fn speed_task_evidence_upload_members_cost_no_requests() {
    let few = upload(24, true);
    let many = upload(96, true);
    assert!(many.packets >= 1, "{many:?}");
    // 72 more files, not one more stat, identity or folder request.
    assert_eq!(few.stats, many.stats, "{few:?} {many:?}");
    assert_eq!(few.identities, many.identities, "{few:?} {many:?}");
    assert_eq!(many.dirs, 0, "{many:?}");
    assert_eq!(few.bulk_dirs, many.bulk_dirs, "{few:?} {many:?}");
}

#[test]
fn speed_task_evidence_upload_without_capability_keeps_its_requests() {
    let few = upload(24, false);
    let many = upload(96, false);
    // Today's sequence: a presence stat before and a stat plus an identity
    // after every member (a file alone costs more, never less).
    assert!(many.stats >= few.stats + 2 * 72, "{few:?} {many:?}");
    assert!(many.identities >= few.identities + 72, "{few:?} {many:?}");
    assert_eq!(many.bulk_dirs, 0, "{many:?}");
    assert_eq!(many.dirs, FOLDERS, "{many:?}");
}

#[test]
fn speed_task_evidence_download_members_cost_no_requests() {
    let few = download(24, true);
    let many = download(96, true);
    assert!(many.packets >= 1, "{many:?}");
    assert_eq!(few.stats, many.stats, "{few:?} {many:?}");
    assert_eq!(few.identities, many.identities, "{few:?} {many:?}");
}

#[test]
fn speed_task_evidence_download_without_capability_keeps_its_requests() {
    let few = download(24, false);
    let many = download(96, false);
    // Today's sequence: the source's stat and identity before each member
    // is published.
    assert!(many.stats >= few.stats + 72, "{few:?} {many:?}");
    assert!(many.identities >= few.identities + 72, "{few:?} {many:?}");
}

#[test]
fn speed_task_bulk_folders_create_a_deep_path_in_one_request() {
    let root = tempfile::tempdir().expect("temp dir");
    let local = root.path().join("local/Vault");
    write(&local.join("a/b/c/deep.txt"), b"deep");
    write(&local.join("a/b/c/deeper.txt"), b"deeper");
    let remote = root.path().join("remote");
    std::fs::create_dir(&remote).expect("remote");
    let (fake, handle) = fake("bulk-folders", true).handle();
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &remote,
        &[&local],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 2);
    assert_eq!(
        std::fs::read(remote.join("Vault/a/b/c/deep.txt")).expect("deep"),
        b"deep"
    );
    let requests = requests(&fake);
    // The new top-level folder keeps its own exclusive creation; the three
    // nested levels below it are one request.
    assert_eq!(requests.dirs, 0, "{requests:?}");
    assert_eq!(requests.bulk_dirs, 1, "{requests:?}");
}

#[test]
fn speed_task_bulk_folders_merge_into_existing_folders() {
    let root = tempfile::tempdir().expect("temp dir");
    let local = root.path().join("local/Vault");
    write(&local.join("a/new.txt"), b"new");
    let remote = root.path().join("remote");
    write(&remote.join("Vault/a/old.txt"), b"old");
    let (fake, handle) = fake("bulk-merge", true).handle();
    let mut merging = job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &remote,
        &[&local],
    );
    merging.folders =
        crate::transfer::FolderConflictPolicy::new(crate::transfer::FolderConflict::Merge);
    let finished = run(merging);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 1);
    // Existing plain folders are merged into, as with single creations.
    assert_eq!(
        std::fs::read(remote.join("Vault/a/old.txt")).expect("old"),
        b"old"
    );
    assert_eq!(
        std::fs::read(remote.join("Vault/a/new.txt")).expect("new"),
        b"new"
    );
    assert_eq!(requests(&fake).dirs, 0);
}
