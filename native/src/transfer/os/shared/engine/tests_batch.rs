//! Packets of small files, server-side copies inside one account, direct
//! streams between connections and the local bridge for connections that
//! cannot read and write at once.
use super::test_backend::{job, run, write, Fake};
use crate::transfer::Endpoint;
use crate::vfs::BatchLimits;
use std::fs;
use std::sync::atomic::Ordering;

const PACKET: BatchLimits = BatchLimits {
    max_files: 64,
    max_bytes: 1024 * 1024,
};

#[test]
fn transfer_engine_task_batch_upload_bundles_small_files() {
    let root = tempfile::tempdir().expect("temp dir");
    let local = root.path().join("local/Vault");
    for index in 0..20 {
        write(
            &local.join(format!("f{index:02}.txt")),
            format!("file {index}").as_bytes(),
        );
    }
    let remote = root.path().join("remote");
    fs::create_dir(&remote).expect("remote");
    let mut fake = Fake::new("put-batch");
    fake.batch = Some(PACKET);
    let (fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &remote,
        &[&local],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 20);
    for index in 0..20 {
        assert_eq!(
            fs::read(remote.join(format!("Vault/f{index:02}.txt"))).expect("published"),
            format!("file {index}").as_bytes()
        );
    }
    assert!(fake.counters.put_batches.load(Ordering::SeqCst) >= 1);
    assert!(
        fake.counters.stages.load(Ordering::SeqCst) < 20,
        "small files travel in packets"
    );
}

#[test]
fn transfer_engine_task_batch_upload_ambiguous_failure_is_never_retried() {
    let root = tempfile::tempdir().expect("temp dir");
    let local = root.path().join("local/Vault");
    for index in 0..6 {
        write(&local.join(format!("f{index}.txt")), b"small");
    }
    let remote = root.path().join("remote");
    fs::create_dir(&remote).expect("remote");
    let mut fake = Fake::new("put-batch-ambiguous");
    fake.batch = Some(PACKET);
    fake.batch_ambiguous = true;
    let (fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Remote(handle),
        &remote,
        &[&local],
    ));
    let unknown = finished
        .issues
        .iter()
        .filter(|issue| issue.message.contains("Ergebnis unbekannt"))
        .count();
    let mut batched = fake.counters.batched.lock().expect("batched").clone();
    let sent = batched.len();
    // Only what went out before the answer was lost has an unknown result;
    // the members never sent go alone (the fixture fails at the second).
    assert!(unknown >= 1, "{:?}", finished.issues);
    assert_eq!(unknown, sent, "{:?}", finished.issues);
    assert_eq!(
        finished.progress.files_done + unknown as u64,
        6,
        "every file is either done alone or of unknown result"
    );
    batched.sort();
    batched.dedup();
    assert_eq!(batched.len(), sent, "no packet member was sent twice");
    assert!(fake.counters.put_batches.load(Ordering::SeqCst) >= 1);
}

#[test]
fn transfer_engine_task_batch_download_writes_parts_and_publishes() {
    let root = tempfile::tempdir().expect("temp dir");
    let remote = root.path().join("remote/Vault");
    for index in 0..12 {
        write(
            &remote.join(format!("f{index:02}.txt")),
            format!("content {index}").as_bytes(),
        );
    }
    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");
    let mut fake = Fake::new("get-batch");
    fake.batch = Some(PACKET);
    let (fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Remote(handle),
        Endpoint::Local,
        &dest,
        &[&remote],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 12);
    for index in 0..12 {
        assert_eq!(
            fs::read(dest.join(format!("Vault/f{index:02}.txt"))).expect("downloaded"),
            format!("content {index}").as_bytes()
        );
    }
    assert!(fake.counters.get_batches.load(Ordering::SeqCst) >= 1);
    assert!(fake.counters.opens.load(Ordering::SeqCst) < 12);
    let parts = fs::read_dir(dest.join("Vault"))
        .expect("listing")
        .filter(|entry| {
            entry
                .as_ref()
                .is_ok_and(|entry| entry.file_name().to_string_lossy().ends_with(".part"))
        })
        .count();
    assert_eq!(parts, 0);
}

#[test]
fn transfer_engine_task_server_copy_inside_one_account() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("remote/Vault");
    write(&source.join("a.txt"), b"alpha");
    write(&source.join("sub/b.txt"), b"beta");
    let dest = root.path().join("remote/copies");
    fs::create_dir_all(&dest).expect("dest");
    let mut fake = Fake::new("server-copy");
    fake.server_copy = true;
    let (fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Remote(handle.clone()),
        Endpoint::Remote(handle),
        &dest,
        &[&source],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 2);
    assert_eq!(
        fs::read(dest.join("Vault/sub/b.txt")).expect("copied"),
        b"beta"
    );
    assert_eq!(fake.counters.server_copies.load(Ordering::SeqCst), 2);
    assert_eq!(
        fake.counters.opens.load(Ordering::SeqCst),
        0,
        "no bytes through us"
    );
}

#[test]
fn transfer_engine_task_failed_server_copy_falls_back_to_streaming() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("remote/Vault");
    write(&source.join("a.txt"), b"alpha");
    write(&source.join("sub/b.txt"), b"beta");
    let dest = root.path().join("remote/copies");
    fs::create_dir_all(&dest).expect("dest");
    let mut fake = Fake::new("server-copy-refused");
    fake.server_copy = true;
    // A locked source on a share: the shortcut fails, the stream decides.
    fake.server_copy_error = Some(std::io::ErrorKind::PermissionDenied);
    let (fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Remote(handle.clone()),
        Endpoint::Remote(handle),
        &dest,
        &[&source],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 2);
    assert_eq!(
        fs::read(dest.join("Vault/a.txt")).expect("copied"),
        b"alpha"
    );
    assert_eq!(fake.counters.server_copies.load(Ordering::SeqCst), 2);
    assert!(
        fake.counters.opens.load(Ordering::SeqCst) > 0,
        "streamed instead"
    );
}

#[test]
fn transfer_engine_task_bridge_when_one_connection_cannot_read_and_write() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("remote/Vault");
    let content = vec![3u8; 700 * 1024];
    write(&source.join("big.bin"), &content);
    write(&source.join("small.txt"), b"small");
    let dest = root.path().join("remote/copies");
    fs::create_dir_all(&dest).expect("dest");
    let mut fake = Fake::new("bridge");
    fake.concurrent = false;
    fake.ceiling = Some(1);
    let (fake, handle) = fake.handle();
    let finished = run(job(
        Endpoint::Remote(handle.clone()),
        Endpoint::Remote(handle),
        &dest,
        &[&source],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(
        fs::read(dest.join("Vault/big.bin")).expect("bridged"),
        content
    );
    assert!(
        !fake.counters.overlap.load(Ordering::SeqCst),
        "a reader and a writer were never open at once"
    );
}

#[test]
fn transfer_engine_task_stream_between_two_connections() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("one/Vault");
    let content: Vec<u8> = (0..900_000u32).map(|index| (index % 253) as u8).collect();
    write(&source.join("big.bin"), &content);
    let dest = root.path().join("two");
    fs::create_dir_all(&dest).expect("dest");
    let (reader, one) = Fake::new("stream-one").handle();
    let (writer, two) = Fake::new("stream-two").handle();
    let finished = run(job(
        Endpoint::Remote(one),
        Endpoint::Remote(two),
        &dest,
        &[&source],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(
        fs::read(dest.join("Vault/big.bin")).expect("streamed"),
        content
    );
    assert_eq!(reader.counters.opens.load(Ordering::SeqCst), 1);
    assert_eq!(writer.counters.stages.load(Ordering::SeqCst), 1);
    assert_eq!(finished.progress.bytes_done, content.len() as u64);
}
