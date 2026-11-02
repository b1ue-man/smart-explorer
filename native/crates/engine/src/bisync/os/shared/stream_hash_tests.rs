//! The sync hash helper: same digest as MD5 over the whole input, a helper
//! thread only beyond the first mebibyte, inline when the system refuses it.
use super::apply_guard::is_drift;
use super::snapshot_hash::md5_to_u64;
use super::stream_hash::{Message, Spawner, StreamHasher, INLINE_BYTES};
use super::transfer_stream::stream;
use std::io::{self, Cursor};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

fn data(len: usize) -> Vec<u8> {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    (0..len)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as u8
        })
        .collect()
}

fn digest_in_chunks(hasher: &mut StreamHasher, input: &[u8], chunk: usize) {
    for part in input.chunks(chunk) {
        hasher.update(part);
    }
}

fn real_spawn(receiver: Receiver<Message>) -> io::Result<JoinHandle<[u8; 16]>> {
    std::thread::Builder::new()
        .name("sync-md5-test".into())
        .spawn(move || drain(receiver))
}

fn drain(receiver: Receiver<Message>) -> [u8; 16] {
    let mut context = md5::Context::new();
    while let Ok(message) = receiver.recv() {
        match message {
            Message::Start(started) => context = *started,
            Message::Data(bytes) => context.consume(&bytes),
        }
    }
    context.compute().0
}

#[test]
fn speed_task_sync_hasher_matches_md5_of_the_whole_input() {
    let mib = INLINE_BYTES;
    for len in [0, 1, mib - 1, mib, mib + 1, 5 * mib] {
        let input = data(len);
        let expected = md5::compute(&input).0;
        for chunk in [7 * 1024, 256 * 1024] {
            let mut hasher = StreamHasher::new();
            digest_in_chunks(&mut hasher, &input, chunk);
            let digest = hasher.finish().expect("digest");
            assert_eq!(digest, expected, "len {len}, chunk {chunk}");
        }
    }
}

static BELOW_SPAWNS: AtomicUsize = AtomicUsize::new(0);

fn counting_below(receiver: Receiver<Message>) -> io::Result<JoinHandle<[u8; 16]>> {
    BELOW_SPAWNS.fetch_add(1, Ordering::SeqCst);
    real_spawn(receiver)
}

#[test]
fn speed_task_sync_hasher_stays_inline_below_one_mebibyte() {
    for len in [0, 1, INLINE_BYTES - 1, INLINE_BYTES] {
        let input = data(len);
        let mut hasher = StreamHasher::with_spawner(counting_below);
        digest_in_chunks(&mut hasher, &input, 7 * 1024);
        assert!(!hasher.uses_helper(), "len {len}");
        assert_eq!(
            hasher.finish().expect("digest"),
            md5::compute(&input).0,
            "len {len}"
        );
    }
    assert_eq!(BELOW_SPAWNS.load(Ordering::SeqCst), 0);
}

static ABOVE_SPAWNS: AtomicUsize = AtomicUsize::new(0);

fn counting_above(receiver: Receiver<Message>) -> io::Result<JoinHandle<[u8; 16]>> {
    ABOVE_SPAWNS.fetch_add(1, Ordering::SeqCst);
    real_spawn(receiver)
}

#[test]
fn speed_task_sync_hasher_switches_to_the_helper_at_one_mebibyte() {
    // The first mebibyte stays inline in 256 KiB steps; the fifth step starts
    // the helper, and exactly one helper serves the whole copy.
    let input = data(INLINE_BYTES + 3 * 256 * 1024);
    let mut hasher = StreamHasher::with_spawner(counting_above);
    let mut steps = input.chunks(256 * 1024);
    for _ in 0..4 {
        hasher.update(steps.next().expect("step"));
        assert!(!hasher.uses_helper());
    }
    for step in steps {
        hasher.update(step);
        assert!(hasher.uses_helper());
    }
    assert_eq!(hasher.finish().expect("digest"), md5::compute(&input).0);
    assert_eq!(ABOVE_SPAWNS.load(Ordering::SeqCst), 1);

    // One byte more than the first mebibyte also switches, inside one update.
    let input = data(INLINE_BYTES + 1);
    let mut hasher = StreamHasher::with_spawner(counting_above);
    hasher.update(&input);
    assert!(hasher.uses_helper());
    assert_eq!(hasher.finish().expect("digest"), md5::compute(&input).0);
}

static REFUSALS: AtomicUsize = AtomicUsize::new(0);

fn refusing(_receiver: Receiver<Message>) -> io::Result<JoinHandle<[u8; 16]>> {
    REFUSALS.fetch_add(1, Ordering::SeqCst);
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "resource temporarily unavailable",
    ))
}

#[test]
fn speed_task_sync_hasher_spawn_failure_stays_inline() {
    let input = data(3 * INLINE_BYTES + 5);
    let mut hasher = StreamHasher::with_spawner(refusing);
    digest_in_chunks(&mut hasher, &input, 256 * 1024);
    assert!(!hasher.uses_helper());
    assert_eq!(hasher.finish().expect("digest"), md5::compute(&input).0);
    // Asked once, then the calling thread keeps the work.
    assert_eq!(REFUSALS.load(Ordering::SeqCst), 1);
}

static GATE: AtomicBool = AtomicBool::new(false);
static ENDED: AtomicBool = AtomicBool::new(false);

fn gated(receiver: Receiver<Message>) -> io::Result<JoinHandle<[u8; 16]>> {
    std::thread::Builder::new().spawn(move || {
        while !GATE.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        let digest = drain(receiver);
        ENDED.store(true, Ordering::SeqCst);
        digest
    })
}

#[test]
fn speed_task_sync_hasher_drop_without_finish_stops_the_helper() {
    let input = data(INLINE_BYTES + 3 * 256 * 1024);
    let mut hasher = StreamHasher::with_spawner(gated);
    digest_in_chunks(&mut hasher, &input, 256 * 1024);
    assert!(hasher.uses_helper());
    // The helper is held back: dropping must neither wait for it ...
    drop(hasher);
    assert!(!ENDED.load(Ordering::SeqCst));
    // ... nor leave it running once it gets to look at the closed channel.
    GATE.store(true, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ENDED.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "helper did not stop");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn panicking(receiver: Receiver<Message>) -> io::Result<JoinHandle<[u8; 16]>> {
    std::thread::Builder::new().spawn(move || {
        let _ = receiver.recv();
        panic!("test helper panic");
    })
}

#[test]
fn speed_task_sync_hasher_helper_panic_is_an_error() {
    let spawner: Spawner = panicking;
    let input = data(3 * INLINE_BYTES);
    let mut hasher = StreamHasher::with_spawner(spawner);
    digest_in_chunks(&mut hasher, &input, 256 * 1024);
    let error = hasher.finish().expect_err("a lost helper is an error");
    assert_eq!(error.kind(), io::ErrorKind::Other);
}

#[test]
fn speed_task_sync_stream_digest_unchanged() {
    let cancel = AtomicBool::new(false);
    for len in [0usize, 1000, 3 * INLINE_BYTES + 17] {
        let input = data(len);
        let expected = md5::compute(&input).0;
        let mut written = Vec::new();
        let mut seen = 0u64;
        let streamed = stream(
            &mut Cursor::new(input.clone()),
            &mut written,
            &cancel,
            None,
            0,
            |bytes| seen += bytes,
        )
        .expect("stream");
        assert_eq!(streamed.digest, expected, "len {len}");
        assert_eq!(streamed.bytes, len as u64);
        assert_eq!(written, input);
        assert_eq!(seen, len as u64);

        // The planned hash is still enforced: right passes, wrong is drift.
        let hash = md5_to_u64(&expected);
        let mut sink = Vec::new();
        stream(
            &mut Cursor::new(input.clone()),
            &mut sink,
            &cancel,
            None,
            hash,
            |_| {},
        )
        .expect("matching planned hash");
        let mut sink = Vec::new();
        let error = stream(
            &mut Cursor::new(input),
            &mut sink,
            &cancel,
            None,
            hash ^ 2,
            |_| {},
        )
        .expect_err("changed content");
        assert!(is_drift(&error), "{error}");
    }
}
