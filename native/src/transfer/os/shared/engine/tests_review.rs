//! Review findings outside of packets: moves that keep folders the target
//! refused, downloads behind a link, fresh uploads and folders at a target
//! that refuses or is too busy, lost folder answers, server copies of a
//! changed source, overwrites of an unreadable file and the breaker.
use super::test_backend::{fwd, job, run, write, Fake};
use crate::transfer::Endpoint;
use crate::types::CopyMode;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

fn stages_in(dir: &Path) -> usize {
    fs::read_dir(dir).map_or(0, |entries| {
        entries
            .filter(|entry| {
                entry
                    .as_ref()
                    .is_ok_and(|entry| entry.file_name().to_string_lossy().contains(".se-upload-"))
            })
            .count()
    })
}

#[test]
fn transfer_engine_task_move_keeps_empty_folders_the_target_refused() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("src/Vault");
    write(&source.join("a.txt"), b"alpha");
    write(&source.join("sub/b.txt"), b"beta");
    fs::create_dir_all(source.join("empty")).expect("empty folder");
    // An existing "Vault" makes the move go file by file; a file named like
    // the empty folder keeps that folder from being created at the target.
    let dest = root.path().join("dest");
    write(&dest.join("Vault/empty"), b"a file, not a folder");
    let mut transfer = job(Endpoint::Local, Endpoint::Local, &dest, &[&source]);
    transfer.mode = CopyMode::Move;
    let finished = run(transfer);
    assert!(
        finished.has_issue("Ziel ist kein Ordner"),
        "{:?}",
        finished.issues
    );
    assert_eq!(finished.progress.files_done, 2);
    assert_eq!(fs::read(dest.join("Vault/a.txt")).expect("a"), b"alpha");
    assert_eq!(fs::read(dest.join("Vault/sub/b.txt")).expect("b"), b"beta");
    assert!(
        source.join("empty").is_dir(),
        "an empty folder the target refused stays at the source"
    );
    assert!(
        !source.join("sub").exists(),
        "an emptied folder whose counterpart exists goes"
    );
}

#[cfg(unix)]
#[test]
fn transfer_engine_task_download_into_a_folder_behind_a_link() {
    let root = tempfile::tempdir().expect("temp dir");
    let remote = root.path().join("remote/Vault");
    write(&remote.join("a.txt"), b"alpha");
    let real = root.path().join("real");
    fs::create_dir(&real).expect("real");
    let alias = root.path().join("alias");
    std::os::unix::fs::symlink(&real, &alias).expect("alias");
    // Like /home → /var/home: the chosen folder lies behind a link.
    let (_fake, handle) = Fake::new("link-ancestor").handle();
    let finished = run(job(
        Endpoint::Remote(handle),
        Endpoint::Local,
        &alias.join("Downloads"),
        &[&remote],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(
        fs::read(real.join("Downloads/Vault/a.txt")).expect("downloaded"),
        b"alpha"
    );

    // Local copies keep refusing a target path through a link.
    let source = root.path().join("local.txt");
    write(&source, b"local");
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Local,
        &alias.join("Copies"),
        &[&source],
    ));
    assert!(finished.has_issue("Link"), "{:?}", finished.issues);
    assert!(!real.join("Copies").exists());
}

#[test]
fn transfer_engine_task_refused_fresh_upload_ends_the_job_with_its_reason() {
    let root = tempfile::tempdir().expect("temp dir");
    let local = root.path().join("local/Vault");
    for index in 0..3 {
        write(&local.join(format!("f{index}.txt")), b"payload");
    }
    let remote = root.path().join("remote");
    fs::create_dir(&remote).expect("remote");
    let mut fake = Fake::new("fresh-full");
    fake.faults.fresh_refused = Some(io::ErrorKind::StorageFull);
    let (_fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &remote,
        &[&local],
    ));
    assert!(
        finished
            .issues
            .first()
            .is_some_and(|issue| issue.message.contains("nimmt keine Dateien mehr an")),
        "{:?}",
        finished.issues
    );
    assert!(
        !finished.has_issue("Ergebnis unbekannt"),
        "{:?}",
        finished.issues
    );
    assert_eq!(finished.progress.files_done, 0);
}

#[test]
fn transfer_engine_task_server_copy_checks_the_source_against_its_listing() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("remote/Vault");
    let note = source.join("note.txt");
    write(&note, b"listed bytes");
    // A fine-grained time, so a later one differs at the listing's grain.
    fs::File::options()
        .write(true)
        .open(&note)
        .expect("note")
        .set_modified(std::time::UNIX_EPOCH + Duration::from_millis(1_700_000_000_123))
        .expect("time");
    let dest = root.path().join("remote/copies");
    fs::create_dir_all(&dest).expect("dest");
    let mut fake = Fake::new("server-copy-changed");
    fake.server_copy = true;
    *fake.faults.rewrite_before_copy.lock().expect("fault") = Some(fwd(&note));
    let (fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Remote(handle.clone()),
        Endpoint::Remote(handle),
        &dest,
        &[&source],
    ));
    assert!(finished.has_issue("geändert"), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 0);
    assert!(
        !dest.join("Vault/note.txt").exists(),
        "a source that changed since its listing is not published"
    );
    assert_eq!(stages_in(&dest.join("Vault")), 0, "its stage is removed");
    assert_eq!(fake.counters.server_copies.load(Ordering::SeqCst), 1);
}

#[test]
fn transfer_engine_task_failing_folders_feed_the_breaker() {
    let root = tempfile::tempdir().expect("temp dir");
    let folders: Vec<PathBuf> = (0..10)
        .map(|index| {
            let folder = root.path().join(format!("local/Vault{index}"));
            write(&folder.join("a.txt"), b"alpha");
            folder
        })
        .collect();
    let remote = root.path().join("remote");
    fs::create_dir(&remote).expect("remote");
    let mut fake = Fake::new("folder-breaker");
    // Refused at once (no retry pause): every folder is a connection failure.
    fake.faults.dir_fault = Some(io::ErrorKind::ConnectionRefused);
    let (_fake, handle) = fake.handle();
    let paths: Vec<&Path> = folders.iter().map(PathBuf::as_path).collect();
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &remote,
        &paths,
    ));
    assert!(
        finished
            .issues
            .first()
            .is_some_and(|issue| issue.message.contains("viele Fehler in Folge")),
        "the reason the job ended comes first: {:?}",
        finished.issues
    );
    assert_eq!(finished.progress.files_done, 0);
}

#[test]
fn transfer_engine_task_lost_folder_answer_leaves_no_empty_twin() {
    let root = tempfile::tempdir().expect("temp dir");
    let local = root.path().join("local/Vault");
    write(&local.join("a.txt"), b"alpha");
    write(&local.join("sub/b.txt"), b"beta");
    let remote = root.path().join("remote");
    fs::create_dir(&remote).expect("remote");
    let fake = Fake::new("folder-answer-lost");
    fake.faults.dir_answer_lost.store(true, Ordering::SeqCst);
    let (_fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &remote,
        &[&local],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 2);
    assert_eq!(fs::read(remote.join("Vault/a.txt")).expect("a"), b"alpha");
    assert_eq!(
        fs::read(remote.join("Vault/sub/b.txt")).expect("b"),
        b"beta"
    );
    assert!(
        !remote.join("Vault (2)").exists(),
        "no numbered twin next to an empty folder"
    );
}

#[test]
fn transfer_engine_task_busy_folder_creation_waits_until_it_succeeds() {
    let root = tempfile::tempdir().expect("temp dir");
    let local = root.path().join("local/Vault");
    write(&local.join("a.txt"), b"alpha");
    write(&local.join("sub/b.txt"), b"beta");
    let remote = root.path().join("remote");
    fs::create_dir(&remote).expect("remote");
    let fake = Fake::new("folder-congestion");
    fake.faults.congested_dirs.store(2, Ordering::SeqCst);
    let (fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &remote,
        &[&local],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 2);
    assert_eq!(
        fs::read(remote.join("Vault/sub/b.txt")).expect("b"),
        b"beta"
    );
    assert_eq!(fake.faults.congested_dirs.load(Ordering::SeqCst), 0);
}

#[test]
fn transfer_engine_task_congestion_waits_until_the_file_arrives() {
    let root = tempfile::tempdir().expect("temp dir");
    let note = root.path().join("local/note.txt");
    write(&note, b"patient");
    let remote = root.path().join("remote");
    fs::create_dir(&remote).expect("remote");
    let fake = Fake::new("congestion");
    fake.faults.congested_stages.store(4, Ordering::SeqCst);
    let (fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &remote,
        &[&note],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 1);
    assert_eq!(
        fs::read(remote.join("note.txt")).expect("arrived"),
        b"patient"
    );
    assert_eq!(
        fake.counters.stages.load(Ordering::SeqCst),
        5,
        "four refusals, then the stage"
    );
}

#[cfg(unix)]
#[test]
fn transfer_engine_task_overwrite_of_an_unreadable_destination_goes_on() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("src/note.txt");
    write(&source, b"new");
    let other = root.path().join("src/other.txt");
    write(&other, b"other");
    let dest = root.path().join("dest");
    write(&dest.join("note.txt"), b"old");
    fs::set_permissions(dest.join("note.txt"), fs::Permissions::from_mode(0o000))
        .expect("unreadable destination");
    let mut transfer = job(Endpoint::Local, Endpoint::Local, &dest, &[&source, &other]);
    transfer.conflict = crate::types::Conflict::Overwrite;
    let finished = run(transfer);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 2);
    assert_eq!(fs::read(dest.join("note.txt")).expect("replaced"), b"new");
    assert_eq!(fs::read(dest.join("other.txt")).expect("copied"), b"other");
}
