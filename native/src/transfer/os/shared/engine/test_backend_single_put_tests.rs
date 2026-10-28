//! A single file as a packet of one (Z-P4) against the fake host: one
//! packet and no stage or promotion; a lost answer is never answered by
//! sending the file again; a host without the capability keeps the single
//! path.
use super::{job, run, write, Fake, Finished};
use crate::transfer::Endpoint;
use crate::vfs::BatchLimits;
use std::sync::atomic::Ordering;
use std::sync::Arc;

const CONTENT: &[u8] = b"a single file";

fn fake(tag: &str, single_put: bool) -> Fake {
    let mut fake = Fake::new(tag);
    fake.identity_proofs = true;
    fake.evidence = true;
    fake.single_put = single_put;
    fake.batch = Some(BatchLimits {
        max_files: 256,
        max_bytes: 16 * 1024 * 1024,
    });
    fake
}

/// Uploads the folder `Vault` with `one.txt` alone in it; the copy is
/// `root/remote/Vault`.
fn upload_one(fake: Fake) -> (Arc<Fake>, Finished, tempfile::TempDir) {
    let root = tempfile::tempdir().expect("temp dir");
    let local = root.path().join("local/Vault");
    write(&local.join("one.txt"), CONTENT);
    let remote = root.path().join("remote");
    std::fs::create_dir(&remote).expect("remote");
    let (fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &remote,
        &[&local],
    ));
    (fake, finished, root)
}

fn remote_names(root: &tempfile::TempDir) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.path().join("remote/Vault"))
        .expect("remote listing")
        .map(|entry| {
            let entry = entry.expect("remote entry");
            entry.file_name().to_string_lossy().into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn speed_task_single_put_one_file_is_one_packet() {
    let (fake, finished, root) = upload_one(fake("single-put", true));
    assert!(finished.errors.is_empty(), "{:?}", finished.errors);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 1);
    let path = root.path().join("remote/Vault/one.txt");
    let published = std::fs::read(path).expect("published");
    assert_eq!(published, CONTENT);
    let counters = &fake.counters;
    assert_eq!(counters.put_batches.load(Ordering::SeqCst), 1);
    assert_eq!(counters.stages.load(Ordering::SeqCst), 0);
    assert_eq!(counters.promotes.load(Ordering::SeqCst), 0);
    assert_eq!(remote_names(&root), ["one.txt"]);
}

#[test]
fn speed_task_single_put_lost_answer_is_never_sent_again() {
    let mut lost = fake("single-put-lost", true);
    lost.faults.packet_answer_lost = true;
    let (fake, finished, root) = upload_one(lost);
    assert_eq!(finished.progress.files_done, 0);
    assert!(
        !finished.errors.is_empty() || !finished.issues.is_empty(),
        "the unknown result is reported"
    );
    let batched = fake.counters.batched.lock().expect("batched").len();
    assert_eq!(batched, 1, "sent exactly once");
    assert_eq!(fake.counters.put_batches.load(Ordering::SeqCst), 1);
    assert_eq!(fake.counters.stages.load(Ordering::SeqCst), 0, "no resend");
    assert_eq!(remote_names(&root), ["one.txt"], "published once");
}

#[test]
fn speed_task_single_put_without_capability_keeps_the_single_path() {
    let (fake, finished, root) = upload_one(fake("single-put-off", false));
    assert!(finished.errors.is_empty(), "{:?}", finished.errors);
    assert_eq!(finished.progress.files_done, 1);
    assert_eq!(fake.counters.put_batches.load(Ordering::SeqCst), 0);
    assert_eq!(fake.counters.promotes.load(Ordering::SeqCst), 1);
    assert_eq!(remote_names(&root), ["one.txt"]);
}
