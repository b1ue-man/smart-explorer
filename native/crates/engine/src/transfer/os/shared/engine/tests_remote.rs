//! Engine jobs against a fake remote: numbered names, never replacing, the
//! one retry before publication, resuming inside a file, cancel, source
//! changes, the breaker, a full target, "transfer missing files" and
//! refused copies into themselves.
use super::test_backend::{collect, fwd, job, run, write, Fake, ReadFault};
use crate::transfer::{Endpoint, ResolvedRoot};
use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn leftovers(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).expect("listing") {
        let entry = entry.expect("entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.contains(".se-upload-") || name.ends_with(".part") {
            found.push(name.clone());
        }
        if entry.file_type().expect("type").is_dir() {
            found.extend(leftovers(&entry.path()));
        }
    }
    found
}

#[test]
fn transfer_engine_task_upload_numbers_taken_root_names_and_never_replaces() {
    let root = tempfile::tempdir().expect("temp dir");
    let vault = root.path().join("local/Vault");
    write(&vault.join("a.txt"), b"alpha");
    write(&vault.join("sub/b.txt"), b"beta");
    let note = root.path().join("local/note.txt");
    write(&note, b"new");
    let remote = root.path().join("remote");
    write(&remote.join("Vault/existing.txt"), b"old");
    write(&remote.join("note.txt"), b"old");
    let (fake, handle) = Fake::new("names").handle();
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &remote,
        &[&vault, &note],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 3);
    assert_eq!(
        fs::read(remote.join("Vault/existing.txt")).expect("old"),
        b"old"
    );
    assert_eq!(fs::read(remote.join("note.txt")).expect("old"), b"old");
    assert_eq!(
        fs::read(remote.join("Vault (2)/a.txt")).expect("a"),
        b"alpha"
    );
    assert_eq!(
        fs::read(remote.join("Vault (2)/sub/b.txt")).expect("b"),
        b"beta"
    );
    assert_eq!(fs::read(remote.join("note (2).txt")).expect("note"), b"new");
    assert_eq!(
        finished.roots,
        vec![
            ResolvedRoot {
                source: fwd(&vault),
                rel: "Vault (2)".to_string()
            },
            ResolvedRoot {
                source: fwd(&note),
                rel: "note (2).txt".to_string()
            },
        ]
    );
    assert!(leftovers(&remote).is_empty(), "{:?}", leftovers(&remote));
    assert_eq!(fake.counters.stages.load(Ordering::SeqCst), 3);
}

#[test]
fn transfer_engine_task_download_retries_once_before_publication() {
    let root = tempfile::tempdir().expect("temp dir");
    let remote = root.path().join("remote/Vault");
    write(&remote.join("a.txt"), b"alpha");
    write(&remote.join("b.txt"), b"beta");
    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");
    let fake = Fake::new("retry");
    fake.read_faults
        .lock()
        .expect("faults")
        .push_back(ReadFault {
            after: None,
            kind: io::ErrorKind::ConnectionReset,
        });
    let (fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Remote(handle),
        Endpoint::Local,
        &dest,
        &[&remote],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 2);
    assert_eq!(fake.counters.opens.load(Ordering::SeqCst), 3, "one retry");
    assert_eq!(fs::read(dest.join("Vault/a.txt")).expect("a"), b"alpha");
    assert!(leftovers(&dest).is_empty());
}

#[test]
fn transfer_engine_task_download_resumes_inside_the_file() {
    let root = tempfile::tempdir().expect("temp dir");
    let content: Vec<u8> = (0..300_000u32).map(|index| (index % 251) as u8).collect();
    let remote = root.path().join("remote/big.bin");
    write(&remote, &content);
    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");
    let mut fake = Fake::new("resume-inside");
    fake.resumable = true;
    fake.read_faults
        .lock()
        .expect("faults")
        .push_back(ReadFault {
            after: Some(100_000),
            kind: io::ErrorKind::ConnectionAborted,
        });
    let (fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Remote(handle),
        Endpoint::Local,
        &dest,
        &[&remote],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(fs::read(dest.join("big.bin")).expect("downloaded"), content);
    assert_eq!(fake.counters.opens.load(Ordering::SeqCst), 1);
    assert_eq!(
        fake.counters.opens_at.load(Ordering::SeqCst),
        1,
        "continued at the offset"
    );
    assert_eq!(finished.progress.bytes_done, content.len() as u64);
    assert!(leftovers(&dest).is_empty());
}

#[test]
fn transfer_engine_task_cancel_removes_the_local_part() {
    let root = tempfile::tempdir().expect("temp dir");
    let remote = root.path().join("remote/slow.bin");
    write(&remote, &vec![7u8; 2 * 1024 * 1024]);
    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");
    let mut fake = Fake::new("cancel");
    fake.read_delay = Duration::from_millis(60);
    let (_fake, handle) = fake.handle();
    let transfer = job(Endpoint::Remote(handle), Endpoint::Local, &dest, &[&remote]);
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = crossbeam_channel::unbounded();
    let worker = {
        let cancel = cancel.clone();
        std::thread::spawn(move || super::run_job(transfer, &tx, &cancel))
    };
    std::thread::sleep(Duration::from_millis(200));
    cancel.store(true, Ordering::Release);
    worker.join().expect("job thread");
    let finished = collect(&rx);
    assert!(finished.canceled);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 0);
    assert!(
        fs::read_dir(&dest).expect("dest").next().is_none(),
        "no part left"
    );
}

#[test]
fn transfer_engine_task_source_changes_are_detected() {
    let root = tempfile::tempdir().expect("temp dir");
    let remote = root.path().join("remote/note.txt");
    write(&remote, b"listed bytes");
    let grown = root.path().join("grown");
    fs::create_dir(&grown).expect("dest");
    let mut fake = Fake::new("grown");
    fake.extra = Some(b"more".to_vec());
    let (_fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Remote(handle),
        Endpoint::Local,
        &grown,
        &[&remote],
    ));
    assert!(finished.has_issue("gewachsen"), "{:?}", finished.issues);
    assert!(fs::read_dir(&grown).expect("dest").next().is_none());

    let wrong = root.path().join("wrong-md5");
    fs::create_dir(&wrong).expect("dest");
    let mut fake = Fake::new("md5");
    fake.md5
        .insert(fwd(&remote), "00000000000000000000000000000000".to_string());
    let (_fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Remote(handle),
        Endpoint::Local,
        &wrong,
        &[&remote],
    ));
    assert!(finished.has_issue("geändert"), "{:?}", finished.issues);
    assert!(fs::read_dir(&wrong).expect("dest").next().is_none());

    let right = root.path().join("right-md5");
    fs::create_dir(&right).expect("dest");
    let mut fake = Fake::new("md5-ok");
    fake.md5
        .insert(fwd(&remote), format!("{:x}", md5::compute(b"listed bytes")));
    let (_fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Remote(handle),
        Endpoint::Local,
        &right,
        &[&remote],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(
        fs::read(right.join("note.txt")).expect("note"),
        b"listed bytes"
    );
}

#[test]
fn transfer_engine_task_breaker_ends_the_job_after_connection_failures() {
    let root = tempfile::tempdir().expect("temp dir");
    let remote = root.path().join("remote/many");
    for index in 0..30 {
        write(&remote.join(format!("f{index:02}.txt")), b"x");
    }
    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");
    let fake = Fake::new("breaker");
    for _ in 0..200 {
        fake.read_faults
            .lock()
            .expect("faults")
            .push_back(ReadFault {
                after: None,
                kind: io::ErrorKind::ConnectionReset,
            });
    }
    let (_fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Remote(handle),
        Endpoint::Local,
        &dest,
        &[&remote],
    ));
    assert!(
        finished.has_issue("viele Fehler in Folge"),
        "{:?}",
        finished.issues
    );
    assert_eq!(finished.progress.files_done, 0);
    assert!(
        finished.issues.len() < 30,
        "the job stopped instead of collecting every failure: {}",
        finished.issues.len()
    );
    assert!(!finished.canceled);
}

#[test]
fn transfer_engine_task_full_target_ends_the_job() {
    let root = tempfile::tempdir().expect("temp dir");
    let local = root.path().join("local/Vault");
    for index in 0..20 {
        write(&local.join(format!("f{index:02}.txt")), b"payload");
    }
    let remote = root.path().join("remote");
    fs::create_dir(&remote).expect("remote");
    let mut fake = Fake::new("full");
    fake.write_fault = Some(io::ErrorKind::StorageFull);
    let (fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &remote,
        &[&local],
    ));
    assert!(
        finished.has_issue("nimmt keine Dateien mehr an"),
        "{:?}",
        finished.issues
    );
    assert_eq!(finished.progress.files_done, 0);
    assert!(
        fake.counters.stages.load(Ordering::SeqCst) < 20,
        "no file after the first"
    );
    assert!(leftovers(&remote).is_empty(), "failed stages are removed");
}

#[test]
fn transfer_engine_task_resume_skips_same_size_and_never_replaces() {
    let root = tempfile::tempdir().expect("temp dir");
    let remote = root.path().join("remote/Vault");
    write(&remote.join("same.txt"), b"12345");
    write(&remote.join("other.txt"), b"abc");
    write(&remote.join("new.txt"), b"n");
    let dest = root.path().join("dest");
    write(&dest.join("Vault/same.txt"), b"54321");
    write(&dest.join("Vault/other.txt"), b"abcdef");
    let (_fake, handle) = Fake::new("resume").handle();
    let mut transfer = job(Endpoint::Remote(handle), Endpoint::Local, &dest, &[&remote]);
    transfer.resume = Some(vec![ResolvedRoot {
        source: fwd(&remote),
        rel: "Vault".to_string(),
    }]);
    let finished = run(transfer);
    assert_eq!(finished.progress.skipped, 1);
    assert_eq!(finished.progress.files_done, 1);
    assert_eq!(finished.issues.len(), 1, "{:?}", finished.issues);
    assert!(finished.has_issue("anderer Größe"));
    assert_eq!(
        fs::read(dest.join("Vault/same.txt")).expect("kept"),
        b"54321"
    );
    assert_eq!(
        fs::read(dest.join("Vault/other.txt")).expect("kept"),
        b"abcdef"
    );
    assert_eq!(fs::read(dest.join("Vault/new.txt")).expect("new"), b"n");
    assert!(!dest.join("Vault (2)").exists(), "no new numbered copy");
}

#[test]
fn transfer_engine_task_same_namespace_over_two_handles_is_refused() {
    let root = tempfile::tempdir().expect("temp dir");
    let remote = root.path().join("remote/Vault");
    write(&remote.join("a.txt"), b"alpha");
    let one = Fake::new("namespace");
    let mut two = Fake::new("namespace-two");
    two.identity = one.identity.clone();
    let (_one, one) = one.handle();
    let (_two, two) = two.handle();
    let below = remote.join("sub");
    let finished = run(job(
        Endpoint::Remote(one),
        Endpoint::Remote(two),
        &below,
        &[&remote],
    ));
    assert!(finished.has_issue("Das Ziel liegt in einer der Quellen"));
    assert!(!below.exists(), "nothing was created");
}

#[test]
fn transfer_engine_task_ambiguous_publication_is_never_retried() {
    let root = tempfile::tempdir().expect("temp dir");
    let note = root.path().join("local/note.txt");
    write(&note, b"once");
    let remote = root.path().join("remote");
    fs::create_dir(&remote).expect("remote");
    let fake = Fake::new("ambiguous");
    *fake.promote_fault_after.lock().expect("fault") = Some(io::ErrorKind::ConnectionReset);
    let (fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &remote,
        &[&note],
    ));
    assert!(
        finished.has_issue("Ergebnis unbekannt"),
        "{:?}",
        finished.issues
    );
    assert_eq!(fake.counters.promotes.load(Ordering::SeqCst), 1);
    assert_eq!(
        fs::read(remote.join("note.txt")).expect("published once"),
        b"once"
    );
    assert!(!remote.join("note (2).txt").exists(), "no duplicate");
}
