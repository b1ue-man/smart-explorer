//! Review findings of packets: a file that changes while its packet is sent
//! is never published, lost packet answers feed the breaker once, a member
//! the peer may have published is never sent again while unsent ones go
//! alone, items changed since the listing go alone with their new length,
//! and a local target that takes nothing ends the job.
use super::test_backend::{job, run, write, Fake};
use super::test_faults::EntryHook;
use crate::transfer::Endpoint;
use crate::vfs::BatchLimits;
use std::fs;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

const PACKET: BatchLimits = BatchLimits {
    max_files: 64,
    max_bytes: 1024 * 1024,
};

fn small_files(dir: &Path, count: usize) {
    for index in 0..count {
        write(
            &dir.join(format!("f{index:02}.txt")),
            format!("file {index}").as_bytes(),
        );
    }
}

fn name_of(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// Every file name below `dir`, with `(2)` copies and stages included.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("listing")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

#[test]
fn transfer_engine_task_packet_never_publishes_a_file_that_changed_while_sent() {
    let root = tempfile::tempdir().expect("temp dir");
    let local = root.path().join("local/Vault");
    small_files(&local, 12);
    let remote = root.path().join("remote");
    fs::create_dir(&remote).expect("remote");
    let mut fake = Fake::new("packet-grows");
    fake.batch = Some(PACKET);
    let grown = Arc::new(Mutex::new(None::<String>));
    let (source, seen) = (local.clone(), grown.clone());
    // The first file of the first packet grows before its bytes are read.
    let hook: EntryHook = Box::new(move |path: &str| {
        let name = name_of(path);
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(source.join(&name))
            .expect("source");
        std::io::Write::write_all(&mut file, b" grown").expect("grow");
        *seen.lock().expect("seen") = Some(name);
    });
    *fake.faults.before_first_entry.lock().expect("hook") = Some(hook);
    let (fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &remote,
        &[&local],
    ));
    assert!(fake.counters.put_batches.load(Ordering::SeqCst) >= 1);
    let grown = grown.lock().expect("grown").clone().expect("a packet ran");
    assert!(finished.has_issue("gewachsen"), "{:?}", finished.issues);
    assert_eq!(finished.issues.len(), 1, "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 11);
    let published = names(&remote.join("Vault"));
    assert!(
        !published.contains(&grown),
        "the changed file is not published: {published:?}"
    );
    assert_eq!(
        published.len(),
        11,
        "every other file once, no stage left: {published:?}"
    );
    let mut batched = fake.counters.batched.lock().expect("batched").clone();
    let sent = batched.len();
    batched.sort();
    batched.dedup();
    assert_eq!(batched.len(), sent, "nothing went twice in a packet");
    assert!(finished.progress.bytes_done <= finished.progress.bytes_total);
}

#[test]
fn transfer_engine_task_lost_packet_answers_feed_the_breaker_once() {
    let root = tempfile::tempdir().expect("temp dir");
    let local = root.path().join("local/Vault");
    small_files(&local, 12);
    let remote = root.path().join("remote");
    fs::create_dir(&remote).expect("remote");
    let mut fake = Fake::new("packet-answer-lost");
    fake.batch = Some(PACKET);
    fake.faults.packet_answer_lost = true;
    let (fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &remote,
        &[&local],
    ));
    assert!(
        !finished.has_issue("viele Fehler in Folge"),
        "one lost answer is one connection failure: {:?}",
        finished.issues
    );
    let unknown = finished
        .issues
        .iter()
        .filter(|issue| issue.message.contains("Ergebnis unbekannt"))
        .count();
    let batched = fake.counters.batched.lock().expect("batched").len();
    assert!(batched >= 2, "a packet ran");
    assert_eq!(unknown, batched, "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done + unknown as u64, 12);
    assert_eq!(
        names(&remote.join("Vault")).len(),
        12,
        "each file exactly once, none again after the lost answer"
    );
    assert!(finished.progress.bytes_done <= finished.progress.bytes_total);
}

#[test]
fn transfer_engine_task_packet_member_the_peer_may_have_published_is_not_sent_again() {
    let root = tempfile::tempdir().expect("temp dir");
    let local = root.path().join("local/Vault");
    small_files(&local, 12);
    let remote = root.path().join("remote");
    fs::create_dir(&remote).expect("remote");
    let mut fake = Fake::new("packet-failed-after-publish");
    fake.batch = Some(PACKET);
    *fake.faults.failed_after_publish.lock().expect("fault") = Some(std::io::ErrorKind::TimedOut);
    let (fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &remote,
        &[&local],
    ));
    assert!(fake.counters.put_batches.load(Ordering::SeqCst) >= 1);
    assert_eq!(finished.issues.len(), 1, "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 11);
    let published = names(&remote.join("Vault"));
    assert_eq!(published.len(), 12, "{published:?}");
    assert!(
        !published.iter().any(|name| name.contains(" (2)")),
        "no duplicate of the member reported as failed: {published:?}"
    );
}

#[test]
fn transfer_engine_task_packet_download_resends_items_changed_since_listing() {
    let root = tempfile::tempdir().expect("temp dir");
    let remote = root.path().join("remote/Vault");
    small_files(&remote, 12);
    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");
    let mut fake = Fake::new("packet-changed-since-listing");
    fake.batch = Some(PACKET);
    fake.faults.grow_first_item.store(true, Ordering::SeqCst);
    let (fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Remote(handle),
        Endpoint::Local,
        &dest,
        &[&remote],
    ));
    assert!(fake.counters.get_batches.load(Ordering::SeqCst) >= 1);
    let grown = fake
        .faults
        .grown
        .lock()
        .expect("grown")
        .clone()
        .expect("a packet ran");
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 12);
    let name = name_of(&grown);
    let expected = fs::read(remote.join(&name)).expect("grown source");
    assert!(expected.ends_with(b" and more"));
    assert_eq!(
        fs::read(dest.join("Vault").join(&name)).expect("downloaded"),
        expected,
        "sent alone with its new length"
    );
    let total: u64 = (0..12)
        .map(|index| {
            fs::metadata(remote.join(format!("f{index:02}.txt")))
                .expect("source")
                .len()
        })
        .sum();
    assert_eq!(finished.progress.bytes_total, total);
    assert_eq!(finished.progress.bytes_done, total);
}

#[cfg(unix)]
#[test]
fn transfer_engine_task_packet_download_into_a_full_target_ends_the_job() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().expect("temp dir");
    let remote = root.path().join("remote/Vault");
    small_files(&remote, 12);
    let dest = root.path().join("dest");
    let locked = dest.join("Vault");
    fs::create_dir_all(&locked).expect("dest");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).expect("read-only");
    if fs::write(locked.join("probe"), b"root").is_ok() {
        // Permissions do not bind this user (root): nothing to observe.
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("restore");
        return;
    }
    let mut fake = Fake::new("packet-target-refuses");
    fake.batch = Some(PACKET);
    let (_fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Remote(handle),
        Endpoint::Local,
        &dest,
        &[&remote],
    ));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("restore");
    assert!(
        finished
            .issues
            .first()
            .is_some_and(|issue| issue.message.contains("nimmt keine Dateien mehr an")),
        "{:?}",
        finished.issues
    );
    assert_eq!(
        finished.issues.len(),
        1,
        "one reason instead of one failure per file: {:?}",
        finished.issues
    );
    assert_eq!(finished.progress.files_done, 0);
}
