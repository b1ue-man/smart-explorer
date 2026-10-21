//! Large files in parts: the file assembled from ranges, one permit doing
//! all parts, the retry from the safe start, cancel, turns between jobs, the
//! fallback to one stream, the change checks and the order of memory and
//! permit. All with the fake remote that reads ranges.
use super::queue::Work;
use super::segments::{self, TRACE};
use super::test_backend::{collect, fwd, job, run, unique, write, Fake, Finished};
use super::{run_job, Engine, FolderRegister, JobView, Side};
use crate::transfer::{Endpoint, JobItems, Layout, TransferJob, TransferKind};
use crate::types::{Conflict, CopyMode};
use crate::vfs::BackendHandle;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

const MIB: usize = 1024 * 1024;
const LIMIT: Duration = Duration::from_secs(120);

fn pattern(len: usize) -> Vec<u8> {
    (0..len)
        .map(|index| ((index * 7 + index / 251) % 251) as u8)
        .collect()
}

fn ranged(tag: &str) -> Fake {
    let mut fake = Fake::new(tag);
    fake.ranges.supported = true;
    fake
}

/// A remote file `big.bin` of `len` bytes and an empty destination.
struct Fixture {
    _root: tempfile::TempDir,
    source: PathBuf,
    dest: PathBuf,
    data: Vec<u8>,
}

impl Fixture {
    fn new(len: usize) -> Self {
        let root = tempfile::tempdir().expect("temp dir");
        let data = pattern(len);
        let source = root.path().join("remote/big.bin");
        write(&source, &data);
        let dest = root.path().join("dest");
        fs::create_dir(&dest).expect("dest");
        Self {
            _root: root,
            source,
            dest,
            data,
        }
    }

    fn job(&self, handle: &BackendHandle) -> TransferJob {
        job(
            Endpoint::Remote(handle.clone()),
            Endpoint::Local,
            &self.dest,
            &[&self.source],
        )
    }

    fn downloaded(&self, name: &str) -> Vec<u8> {
        fs::read(self.dest.join(name)).expect("the downloaded file")
    }

    fn entries(&self) -> usize {
        fs::read_dir(&self.dest).expect("dest").count()
    }
}

/// Runs a job on its own thread: one that does not end in time failed.
fn run_within(transfer: TransferJob, limit: Duration) -> Finished {
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done_tx.send(run(transfer));
    });
    done_rx
        .recv_timeout(limit)
        .expect("the job finished instead of waiting forever")
}

fn offsets(fake: &Fake) -> Vec<u64> {
    let requests = fake.ranges.requests.lock().expect("fixture lock");
    requests.iter().map(|(offset, _)| *offset).collect()
}

fn mib(count: usize) -> u64 {
    (count * MIB) as u64
}

#[test]
fn speed_task_segments_download_assembles_the_file_from_ranges() {
    let fixture = Fixture::new(40 * MIB);
    let mut fake = ranged("assemble");
    fake.ceiling = Some(3);
    fake.read_delay = Duration::from_millis(3);
    let (fake, handle) = fake.handle();
    let finished = run_within(fixture.job(&handle), LIMIT);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 1);
    assert_eq!(finished.progress.bytes_done, mib(40));
    assert!(fixture.downloaded("big.bin") == fixture.data, "content");
    let mut asked = offsets(&fake);
    asked.sort_unstable();
    let expected: Vec<u64> = (0..5).map(|part| mib(part * 8)).collect();
    assert_eq!(asked, expected, "five parts of the shortest length");
    assert!(
        fake.counters.max_active.load(Ordering::SeqCst) >= 2,
        "a helper took parts next to the owner"
    );
    assert_eq!(
        fake.counters.opens.load(Ordering::SeqCst),
        0,
        "no one stream"
    );
    assert_eq!(fixture.entries(), 1, "no part file is left");
}

#[test]
fn speed_task_segments_single_permit_owner_does_all_ranges() {
    let fixture = Fixture::new(40 * MIB);
    let mut fake = ranged("single");
    fake.ceiling = Some(1);
    let (fake, handle) = fake.handle();
    let finished = run_within(fixture.job(&handle), LIMIT);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert!(fixture.downloaded("big.bin") == fixture.data, "content");
    let expected: Vec<u64> = (0..5).map(|part| mib(part * 8)).collect();
    assert_eq!(offsets(&fake), expected, "one after the other, in order");
    assert_eq!(fake.counters.max_active.load(Ordering::SeqCst), 1);
}

#[test]
fn speed_task_segments_retry_resumes_at_prefix() {
    let fixture = Fixture::new(40 * MIB);
    let mut fake = ranged("retry");
    fake.ceiling = Some(1);
    fake.ranges.fail_once =
        std::sync::Mutex::new(Some((mib(16), MIB, io::ErrorKind::ConnectionReset)));
    let (fake, handle) = fake.handle();
    let finished = run_within(fixture.job(&handle), LIMIT);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 1);
    assert_eq!(
        finished.progress.bytes_done,
        mib(40),
        "no byte counted twice"
    );
    assert!(fixture.downloaded("big.bin") == fixture.data, "content");
    assert_eq!(
        offsets(&fake),
        vec![0, mib(8), mib(16), mib(16), mib(24), mib(32)],
        "the retry starts where every earlier byte was written"
    );
}

#[test]
fn speed_task_segments_cancel_stops_helpers() {
    let fixture = Fixture::new(96 * MIB);
    let mut fake = ranged("cancel");
    fake.ceiling = Some(3);
    fake.read_delay = Duration::from_millis(4);
    let (fake, handle) = fake.handle();
    let cancel = Arc::new(AtomicBool::new(false));
    let (done_tx, done_rx) = mpsc::channel();
    let transfer = fixture.job(&handle);
    let flag = cancel.clone();
    std::thread::spawn(move || {
        let (tx, rx) = crossbeam_channel::unbounded();
        run_job(transfer, &tx, &flag);
        let _ = done_tx.send(collect(&rx));
    });
    let began = Instant::now();
    while fake
        .ranges
        .requests
        .lock()
        .expect("fixture lock")
        .is_empty()
    {
        assert!(began.elapsed() < LIMIT, "the first part began");
        std::thread::sleep(Duration::from_millis(5));
    }
    cancel.store(true, Ordering::Release);
    let finished = done_rx
        .recv_timeout(Duration::from_secs(60))
        .expect("the canceled job ended");
    assert!(finished.canceled);
    assert_eq!(finished.progress.files_done, 0);
    assert!(
        fake.ranges.requests.lock().expect("fixture lock").len() < 12,
        "the parts that were not begun stay unbegun"
    );
    assert_eq!(fake.counters.active.load(Ordering::SeqCst), 0, "all closed");
    assert_eq!(fixture.entries(), 0, "no part file, nothing published");
}

#[test]
fn speed_task_segments_round_robin_with_small_job() {
    let big = Fixture::new(96 * MIB);
    let small_root = tempfile::tempdir().expect("temp dir");
    let small = small_root.path().join("remote/small.txt");
    write(&small, b"small");
    let small_dest = small_root.path().join("dest");
    fs::create_dir(&small_dest).expect("dest");
    let mut fake = ranged("turns");
    fake.ceiling = Some(2);
    fake.read_delay = Duration::from_millis(4);
    let (_fake, handle) = fake.handle();
    let big_job = big.job(&handle);
    let big_run = std::thread::spawn(move || {
        let finished = run(big_job);
        (finished, Instant::now())
    });
    std::thread::sleep(Duration::from_millis(150));
    let small_job = job(
        Endpoint::Remote(handle.clone()),
        Endpoint::Local,
        &small_dest,
        &[&small],
    );
    let small_run = std::thread::spawn(move || {
        let finished = run(small_job);
        (finished, Instant::now())
    });
    let (small_done, small_at) = small_run.join().expect("small job");
    let (big_done, big_at) = big_run.join().expect("big job");
    assert!(small_done.issues.is_empty(), "{:?}", small_done.issues);
    assert!(big_done.issues.is_empty(), "{:?}", big_done.issues);
    assert_eq!(
        fs::read(small_dest.join("small.txt")).expect("small"),
        b"small"
    );
    assert!(
        small_at + Duration::from_millis(200) < big_at,
        "the small file took a permit between the parts of the big one"
    );
}

#[test]
fn speed_task_segments_fallback_when_server_ignores_ranges() {
    let fixture = Fixture::new(20 * MIB);
    let mut fake = ranged("ignored");
    fake.ranges.ignored = true;
    fake.ceiling = Some(1);
    let (fake, handle) = fake.handle();
    let first = run_within(fixture.job(&handle), LIMIT);
    assert!(
        first.issues.is_empty(),
        "no error for the fallback: {:?}",
        first.issues
    );
    assert_eq!(first.progress.bytes_done, mib(20));
    assert!(fixture.downloaded("big.bin") == fixture.data, "content");
    assert_eq!(fake.ranges.refused.load(Ordering::SeqCst), 1, "asked once");
    assert_eq!(fake.counters.opens.load(Ordering::SeqCst), 1, "one stream");
    let second = run_within(fixture.job(&handle), LIMIT);
    assert!(second.issues.is_empty(), "{:?}", second.issues);
    assert!(fixture.downloaded("big (2).bin") == fixture.data, "content");
    assert_eq!(
        fake.ranges.refused.load(Ordering::SeqCst),
        1,
        "the connection is remembered: the second job does not ask again"
    );
    assert_eq!(fake.counters.opens.load(Ordering::SeqCst), 2);
}

#[test]
fn speed_task_segments_short_range_reports_source_changed() {
    let fixture = Fixture::new(20 * MIB);
    let mut fake = ranged("short");
    fake.ranges.short_by = 1000;
    fake.ceiling = Some(1);
    let (_fake, handle) = fake.handle();
    let finished = run_within(fixture.job(&handle), LIMIT);
    assert!(
        finished.has_issue("Quelle wurde während der Übertragung geändert"),
        "{:?}",
        finished.issues
    );
    assert_eq!(finished.progress.files_done, 0);
    assert_eq!(fixture.entries(), 0, "nothing published, no part left");
}

#[test]
fn speed_task_segments_md5_prefix_hasher_matches_single_stream() {
    let fixture = Fixture::new(40 * MIB);
    let listed = format!("{:x}", md5::compute(&fixture.data));
    let path = fwd(&fixture.source);
    // One stream: the reference.
    let mut plain = Fake::new("md5-plain");
    plain.md5.insert(path.clone(), listed.clone());
    let (plain, handle) = plain.handle();
    let finished = run_within(fixture.job(&handle), LIMIT);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(plain.counters.opens.load(Ordering::SeqCst), 1);
    // Parts, three at a time: the same hash is accepted.
    let mut parts = ranged("md5-parts");
    parts.ceiling = Some(3);
    parts.read_delay = Duration::from_millis(2);
    parts.md5.insert(path.clone(), listed);
    let (parts, handle) = parts.handle();
    let finished = run_within(fixture.job(&handle), LIMIT);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(offsets(&parts).len(), 5);
    assert!(fixture.downloaded("big (2).bin") == fixture.data, "content");
    // A listed hash that is not the content is caught in parts too.
    let mut wrong = ranged("md5-wrong");
    wrong.ceiling = Some(3);
    wrong.md5.insert(path, "0".repeat(32));
    let (_wrong, handle) = wrong.handle();
    let finished = run_within(fixture.job(&handle), LIMIT);
    assert!(
        finished.has_issue("Quelle wurde während der Übertragung geändert"),
        "{:?}",
        finished.issues
    );
    assert_eq!(fixture.entries(), 2, "the third file was not published");
}

#[test]
fn speed_task_segments_publish_waits_for_all_ranges() {
    let fixture = Fixture::new(40 * MIB);
    let mut fake = ranged("publish");
    fake.ceiling = Some(3);
    fake.read_delay = Duration::from_millis(3);
    let (_fake, handle) = fake.handle();
    let finished = Arc::new(AtomicBool::new(false));
    let target = fixture.dest.join("big.bin");
    let watcher = {
        let (finished, data) = (finished.clone(), fixture.data.clone());
        std::thread::spawn(move || {
            let mut seen = None;
            while !finished.load(Ordering::Acquire) {
                if target.exists() {
                    seen = Some(fs::read(&target).map(|content| content == data));
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            seen
        })
    };
    let result = run_within(fixture.job(&handle), LIMIT);
    finished.store(true, Ordering::Release);
    assert!(result.issues.is_empty(), "{:?}", result.issues);
    let seen = watcher.join().expect("watcher");
    assert!(
        matches!(seen, Some(Ok(true))),
        "the name appeared only with every byte in place: {seen:?}"
    );
}

#[test]
fn speed_task_segments_ticket_without_work_is_noop() {
    let root = tempfile::tempdir().expect("temp dir");
    let target = fwd(root.path());
    let items = JobItems::Roots {
        paths: Vec::new(),
        base: None,
    };
    let view = JobView {
        source: Side::Local,
        target: Side::Local,
        target_dir: &target,
        items: &items,
        layout: Layout::Tree,
        filter: None,
        conflict: Conflict::Rename,
        mode: CopyMode::Copy,
        source_label: "Quelle",
        target_label: "Ziel",
        resume: None,
    };
    let flow = crate::transfer::flow(unique("ticket"), None);
    let folders = FolderRegister::new(Side::Local, &target, flow);
    let plan = super::roots::RootPlan::default();
    let cancel = AtomicBool::new(false);
    let report = |_: crate::transfer::TransferMsg| {};
    let engine = Engine::new(view, TransferKind::Local, folders, &cancel, &report, &plan);
    let mut buffer = vec![0u8; 1024];
    segments::help(&engine, 4711, &mut buffer);
    engine.queue.finish_producing();
    assert!(engine
        .queue
        .push(Work::Segment { job: 4711 }, engine.stop_flag()));
    assert_eq!(
        engine.queue.view().queued,
        1,
        "the ticket waits in the queue"
    );
    let slot = engine.queue.add_worker();
    super::worker::run(&engine, slot);
    assert_eq!(
        engine.queue.view().queued,
        0,
        "a worker took it and went on"
    );
    assert_eq!(engine.stats.running(), 0);
    assert_eq!(engine.issues.total(), 0);
    assert!(!engine.stopped());
}

#[test]
fn speed_task_segments_memory_is_reserved_before_the_permit() {
    let fixture = Fixture::new(40 * MIB);
    let mut fake = ranged("order");
    fake.ceiling = Some(3);
    fake.read_delay = Duration::from_millis(3);
    let (_fake, handle) = fake.handle();
    let before = TRACE.lock().expect("trace").len();
    let finished = run_within(fixture.job(&handle), LIMIT);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    let events = TRACE.lock().expect("trace").clone();
    assert!(
        events[before..].iter().any(|(_, event)| *event == "permit"),
        "a helper took a permit"
    );
    let mut last: std::collections::HashMap<_, &str> = std::collections::HashMap::new();
    for (thread, event) in &events {
        let previous = last.insert(*thread, *event);
        if *event == "permit" {
            assert_eq!(previous, Some("memory"), "memory comes first");
        }
        assert!(
            !(*event == "memory" && previous == Some("permit")),
            "no memory is awaited while a permit is held"
        );
    }
}

#[test]
fn speed_task_segments_name_taken_is_numbered_not_replaced() {
    let fixture = Fixture::new(24 * MIB);
    fs::write(fixture.dest.join("big.bin"), b"older file").expect("existing");
    let mut fake = ranged("taken");
    fake.ceiling = Some(2);
    let (fake, handle) = fake.handle();
    let finished = run_within(fixture.job(&handle), LIMIT);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(fixture.downloaded("big.bin"), b"older file", "kept");
    assert!(
        fixture.downloaded("big (2).bin") == fixture.data,
        "numbered"
    );
    assert!(!offsets(&fake).is_empty(), "transferred in parts");
    assert_eq!(fixture.entries(), 2);
}
