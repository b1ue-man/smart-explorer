//! Remote → remote packets (G3-13): byte-exact copies between two packet
//! connections, no replacement of a destination that appears while the
//! packet is published, a source that changed since the listing, no packet
//! inside one account, and the benchmark against single files.
use super::super::test_backend::{fwd, job, run, write, Fake};
use super::super::test_faults::EntryHook;
use super::packet_limits;
use crate::transfer::Endpoint;
use crate::vfs::BatchLimits;
use std::fs;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const PACKET: BatchLimits = BatchLimits {
    max_files: 64,
    max_bytes: 1024 * 1024,
};

fn small_files(dir: &Path, count: usize) {
    for index in 0..count {
        write(
            &dir.join(format!("f{index:02}.txt")),
            format!("remote file {index}").as_bytes(),
        );
    }
}

fn packet_fake(tag: &str) -> Fake {
    let mut fake = Fake::new(tag);
    fake.batch = Some(PACKET);
    fake
}

#[test]
fn remote_packet_task_limits_are_what_both_sides_take() {
    let from = BatchLimits {
        max_files: 256,
        max_bytes: 4 << 20,
    };
    let to = BatchLimits {
        max_files: 64,
        max_bytes: 16 << 20,
    };
    assert_eq!(
        packet_limits(from, to),
        BatchLimits {
            max_files: 64,
            max_bytes: 4 << 20,
        }
    );
}

#[test]
fn remote_packet_task_copies_small_files_byte_exact_in_packets() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("source/Vault");
    small_files(&source, 20);
    let target = root.path().join("target");
    fs::create_dir(&target).expect("target");
    let (from, from_handle) = packet_fake("rr-source").handle();
    let (to, to_handle) = packet_fake("rr-target").handle();
    let finished = run(job(
        Endpoint::Remote(from_handle),
        Endpoint::Remote(to_handle),
        &target,
        &[&source],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 20);
    assert_eq!(finished.progress.bytes_done, finished.progress.bytes_total);
    for index in 0..20 {
        assert_eq!(
            fs::read(target.join(format!("Vault/f{index:02}.txt"))).expect("published"),
            format!("remote file {index}").as_bytes(),
            "byte-exact"
        );
    }
    assert!(from.counters.get_batches.load(Ordering::SeqCst) >= 1);
    assert!(to.counters.put_batches.load(Ordering::SeqCst) >= 1);
    assert!(
        from.counters.opens.load(Ordering::SeqCst) < 20,
        "no read stream per small file"
    );
    assert!(
        to.counters.stages.load(Ordering::SeqCst) < 20,
        "no stage per small file"
    );
    let mut batched = to.counters.batched.lock().expect("batched").clone();
    let sent = batched.len();
    batched.sort();
    batched.dedup();
    assert_eq!(batched.len(), sent, "nothing went twice in a packet");
}

#[test]
fn remote_packet_task_never_replaces_a_destination_that_appeared() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("source/Vault");
    small_files(&source, 12);
    let target = root.path().join("target");
    fs::create_dir(&target).expect("target");
    let (_from, from_handle) = packet_fake("rr-no-replace-source").handle();
    let to = packet_fake("rr-no-replace-target");
    let raced = Arc::new(Mutex::new(None::<String>));
    let seen = raced.clone();
    // Another program creates the first member's destination just before
    // the target publishes the packet.
    let hook: EntryHook = Box::new(move |path: &str| {
        fs::write(path, b"foreign").expect("foreign file");
        *seen.lock().expect("seen") = Some(path.to_string());
    });
    *to.faults.before_first_entry.lock().expect("hook") = Some(hook);
    let (to, to_handle) = to.handle();
    let finished = run(job(
        Endpoint::Remote(from_handle),
        Endpoint::Remote(to_handle),
        &target,
        &[&source],
    ));
    let raced = raced.lock().expect("raced").clone().expect("a packet ran");
    assert_eq!(
        fs::read(&raced).expect("foreign file stays"),
        b"foreign",
        "the destination that appeared is never replaced"
    );
    // That member's result is unknown: reported once, never sent again.
    assert_eq!(finished.issues.len(), 1, "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 11);
    let mut batched = to.counters.batched.lock().expect("batched").clone();
    let sent = batched.len();
    batched.sort();
    batched.dedup();
    assert_eq!(batched.len(), sent, "nothing went twice");
    for index in 0..12 {
        let path = target.join(format!("Vault/f{index:02}.txt"));
        if fwd(&path) == raced {
            continue;
        }
        assert_eq!(
            fs::read(&path).expect("published"),
            format!("remote file {index}").as_bytes()
        );
    }
}

#[test]
fn remote_packet_task_changed_source_goes_alone_with_its_new_length() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("source/Vault");
    small_files(&source, 12);
    let target = root.path().join("target");
    fs::create_dir(&target).expect("target");
    let from = packet_fake("rr-grown-source");
    from.faults.grow_first_item.store(true, Ordering::SeqCst);
    let (from, from_handle) = from.handle();
    let (_to, to_handle) = packet_fake("rr-grown-target").handle();
    let finished = run(job(
        Endpoint::Remote(from_handle),
        Endpoint::Remote(to_handle),
        &target,
        &[&source],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 12);
    let grown = from.faults.grown.lock().expect("grown").clone();
    let grown = grown.expect("the first packet item grew");
    let name = grown.rsplit('/').next().expect("name").to_string();
    let copied = fs::read(target.join("Vault").join(&name)).expect("published");
    assert!(copied.ends_with(b" and more"), "copied with its new length");
}

#[test]
fn remote_packet_task_inside_one_account_forms_no_packet() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("source/Vault");
    small_files(&source, 8);
    let target = root.path().join("target");
    fs::create_dir(&target).expect("target");
    let (fake, handle) = packet_fake("rr-same-account").handle();
    let finished = run(job(
        Endpoint::Remote(handle.clone()),
        Endpoint::Remote(handle),
        &target,
        &[&source],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 8);
    assert_eq!(fake.counters.get_batches.load(Ordering::SeqCst), 0);
    assert_eq!(fake.counters.put_batches.load(Ordering::SeqCst), 0);
}

/// Copies `files` files of 4 KiB between two connections and prints the
/// rate; `packets` switches the packet support of both sides.
fn bench(case: &str, files: usize, packets: bool) {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("source/tree");
    let bytes = vec![0xA5u8; 4096];
    for index in 0..files {
        write(
            &source.join(format!("d{:02}/f{index:05}.bin", index % 20)),
            &bytes,
        );
    }
    let target = root.path().join("target");
    fs::create_dir(&target).expect("target");
    let mut from = Fake::new("rr-bench-source");
    let mut to = Fake::new("rr-bench-target");
    if packets {
        from.batch = Some(PACKET);
        to.batch = Some(PACKET);
    }
    let (_from, from_handle) = from.handle();
    let (_to, to_handle) = to.handle();
    let started = Instant::now();
    let finished = run(job(
        Endpoint::Remote(from_handle),
        Endpoint::Remote(to_handle),
        &target,
        &[&source],
    ));
    let seconds = started.elapsed().as_secs_f64().max(1e-9);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, files as u64);
    println!("BENCH {case} files_per_s={:.0}", files as f64 / seconds);
}

#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture"]
fn speed_task_remote_to_remote_packets_2000_small_files() {
    bench("remote_to_remote_packets", 2000, true);
}

#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture"]
fn speed_task_remote_to_remote_single_files_2000_small_files() {
    bench("remote_to_remote_single_files", 2000, false);
}
