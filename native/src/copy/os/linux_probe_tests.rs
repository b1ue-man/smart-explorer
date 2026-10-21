use super::*;
use std::collections::HashSet;
use std::os::unix::fs::FileExt;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread::ThreadId;

const MIB: u64 = 1024 * 1024;

/// One kernel call the stand-in saw: which stage, which thread, which bytes.
struct Call {
    stage: i32,
    thread: ThreadId,
    from: u64,
    length: usize,
}

static CALLS: Mutex<Vec<Call>> = Mutex::new(Vec::new());

fn calls_of(stage: i32) -> Vec<(ThreadId, u64, usize)> {
    let calls = CALLS.lock().unwrap_or_else(|p| p.into_inner());
    let mut found: Vec<_> = calls
        .iter()
        .filter(|call| call.stage == stage)
        .map(|call| (call.thread, call.from, call.length))
        .collect();
    found.sort_by_key(|&(_, from, _)| from);
    found
}

/// Copies the bytes with `pread`/`pwrite` and writes down who did what.
fn recording_call(
    reader: &File,
    writer: &File,
    from: u64,
    to: u64,
    length: usize,
) -> io::Result<usize> {
    let mut buffer = vec![0u8; length];
    let read = reader.read_at(&mut buffer, from)?;
    writer.write_all_at(&buffer[..read], to)?;
    CALLS.lock().unwrap_or_else(|p| p.into_inner()).push(Call {
        stage: writer.as_raw_fd(),
        thread: std::thread::current().id(),
        from,
        length: read,
    });
    Ok(read)
}

fn spawn_always(_: usize) -> bool {
    true
}

/// The two ways of copying a span, as the probe asks for them; both copy with
/// the recording stand-in so the content can be compared, and write down which
/// way was asked for and with how many threads.
struct Recorder<'a> {
    reader: &'a File,
    writer: &'a File,
    cancel: &'a AtomicBool,
    hooks: Hooks,
    progress: Vec<u64>,
    asked: Vec<(&'static str, u64, u64, usize)>,
}

impl Recorder<'_> {
    fn copy(&mut self, from: u64, to: u64, threads: usize) -> io::Result<Option<u64>> {
        let progress = &mut self.progress;
        copy_span_with(
            self.reader,
            self.writer,
            from,
            to,
            threads,
            self.cancel,
            &mut |bytes| progress.push(bytes),
            &self.hooks,
        )
    }
}

impl Spans for Recorder<'_> {
    fn direct(&mut self, from: u64, to: u64, threads: usize) -> io::Result<Option<u64>> {
        self.asked.push(("direct", from, to, threads));
        self.copy(from, to, threads)
    }

    fn buffered(&mut self, from: u64, to: u64, threads: usize) -> io::Result<Option<u64>> {
        self.asked.push(("buffered", from, to, threads));
        self.copy(from, to, threads)
    }
}

/// The parallel section took half the time of the sequential one.
fn twice_as_fast(phase: Phase, _: Duration) -> Duration {
    match phase {
        Phase::Sequential => Duration::from_millis(1000),
        Phase::Parallel => Duration::from_millis(500),
    }
}

/// Two threads were slower than one.
fn slower(phase: Phase, _: Duration) -> Duration {
    match phase {
        Phase::Sequential => Duration::from_millis(500),
        Phase::Parallel => Duration::from_millis(1000),
    }
}

fn recording_hooks() -> Hooks {
    Hooks {
        call: recording_call,
        may_spawn: spawn_always,
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    reader: File,
    writer: File,
    stage: std::path::PathBuf,
    content: Vec<u8>,
}

fn fixture(source_len: u64, stage_len: u64) -> Fixture {
    let dir = tempfile::tempdir().expect("temp dir");
    // Every mebibyte differs from its neighbours, so a section that lands at
    // the wrong offset changes the file.
    let content: Vec<u8> = (0..source_len)
        .map(|i| (i.wrapping_mul(31) ^ (i >> 10) ^ (i >> 20)) as u8)
        .collect();
    let source = dir.path().join("source.bin");
    std::fs::write(&source, &content).expect("source");
    let stage = dir.path().join("stage.bin");
    let writer = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&stage)
        .expect("stage");
    writer.set_len(stage_len).expect("sparse stage");
    CALLS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .retain(|call| call.stage != writer.as_raw_fd());
    Fixture {
        reader: File::open(&source).expect("reader"),
        writer,
        stage,
        content,
        _dir: dir,
    }
}

/// A 40 MiB file cut into sections of 4 MiB: 4 MiB with one thread, 4 MiB
/// with two, 32 MiB (four transfer parts) for the rest.
const SIZE: u64 = 40 * MIB;
const SECTION: u64 = 4 * MIB;

struct Outcome {
    result: io::Result<Option<u64>>,
    progress: Vec<u64>,
    verdicts: Vec<bool>,
    asked: Vec<(&'static str, u64, u64, usize)>,
}

fn run(
    f: &Fixture,
    threads: usize,
    cancel: &AtomicBool,
    hooks: Hooks,
    elapsed: fn(Phase, Duration) -> Duration,
) -> Outcome {
    let sections = Sections {
        from: 0,
        end: SIZE,
        section: SECTION,
        threads,
    };
    let verdicts = Mutex::new(Vec::new());
    let report = |helps: bool| verdicts.lock().expect("verdicts").push(helps);
    let mut spans = Recorder {
        reader: &f.reader,
        writer: &f.writer,
        cancel,
        hooks,
        progress: Vec::new(),
        asked: Vec::new(),
    };
    let result = probe_sections(&sections, &mut spans, &report, elapsed);
    Outcome {
        result,
        progress: spans.progress,
        asked: spans.asked,
        verdicts: verdicts.into_inner().expect("verdicts"),
    }
}

fn threads_in(calls: &[(ThreadId, u64, usize)], from: u64, to: u64) -> usize {
    calls
        .iter()
        .filter(|&&(_, at, _)| at >= from && at < to)
        .map(|&(thread, _, _)| thread)
        .collect::<HashSet<_>>()
        .len()
}

#[test]
fn speed_task_local_probe_section_len_scales_and_needs_room() {
    assert_eq!(section_len(128 * MIB - 1), None, "too short to probe");
    assert_eq!(section_len(0), None);
    // A file of 128 MiB (the engine's PROBE_FROM) gets the shortest section.
    assert_eq!(section_len(128 * MIB), Some(32 * MIB));
    assert_eq!(section_len(256 * MIB), Some(32 * MIB), "one eighth");
    assert_eq!(
        section_len(384 * MIB + 5),
        Some(48 * MIB),
        "whole mebibytes"
    );
    assert_eq!(section_len(512 * MIB), Some(64 * MIB));
    assert_eq!(section_len(4096 * MIB), Some(64 * MIB), "capped at 64 MiB");
    // Two sections are never more than half of the file.
    for size in [128 * MIB, 200 * MIB, 513 * MIB, 4096 * MIB, 100_000 * MIB] {
        let section = section_len(size).expect("probed");
        assert!(2 * section <= size / 2, "{size}");
    }
}

#[test]
fn speed_task_local_probe_parallel_needs_ten_percent_more_throughput() {
    let seq = (64 * MIB, Duration::from_millis(1000));
    let par = |millis| (64 * MIB, Duration::from_millis(millis));
    assert!(parallel_helps(seq, par(500)), "twice as fast");
    assert!(parallel_helps(seq, par(909)), "10.01 % more");
    assert!(!parallel_helps(seq, par(910)), "9.89 % more");
    assert!(!parallel_helps(seq, par(1000)), "equal");
    assert!(!parallel_helps(seq, par(1500)), "slower");
    // Exactly 10 % counts: 1100 bytes in 1000 ms against 1000 bytes.
    assert!(parallel_helps(
        (1000, Duration::from_secs(1)),
        (1100, Duration::from_secs(1))
    ));
    assert!(!parallel_helps(
        (1000, Duration::from_secs(1)),
        (1099, Duration::from_secs(1))
    ));
    // A section of other bytes is compared by rate, not by time.
    assert!(parallel_helps(
        (100, Duration::from_secs(1)),
        (200, Duration::from_millis(1800))
    ));
    // Clocks that read zero never divide by it.
    assert!(!parallel_helps((1, Duration::ZERO), (1, Duration::ZERO)));
    assert!(parallel_helps(
        (1, Duration::from_secs(1)),
        (1, Duration::ZERO)
    ));
}

#[test]
fn speed_task_local_probe_sections_and_rest_land_at_their_offsets_when_parallel_helps() {
    let f = fixture(SIZE, SIZE);
    let never = AtomicBool::new(false);
    let done = run(&f, 8, &never, recording_hooks(), twice_as_fast);
    assert_eq!(done.result.expect("copy"), Some(SIZE));
    assert!(
        std::fs::read(&f.stage).expect("stage") == f.content,
        "the stage differs from the source"
    );
    assert_eq!(done.verdicts, [true], "one verdict, reported once");
    // Every byte counted once.
    assert_eq!(done.progress.iter().sum::<u64>(), SIZE);
    // The rest runs direct as well, with the full count limited to one thread
    // per transfer part (32 MiB = 4).
    assert_eq!(
        done.asked,
        [
            ("direct", 0, SECTION, 1),
            ("direct", SECTION, 2 * SECTION, 2),
            ("direct", 2 * SECTION, SIZE, 4),
        ]
    );
    let calls = calls_of(f.writer.as_raw_fd());
    let mut next = 0;
    for &(_, from, length) in &calls {
        assert_eq!(from, next, "ranges neither overlap nor leave a gap");
        next += length as u64;
    }
    assert_eq!(next, SIZE);
    assert_eq!(threads_in(&calls, 0, SECTION), 1);
    assert_eq!(threads_in(&calls, SECTION, 2 * SECTION), 2);
    assert_eq!(threads_in(&calls, 2 * SECTION, SIZE), 4);
}

#[test]
fn speed_task_local_probe_rest_is_one_buffered_stream_when_parallel_does_not_help() {
    let f = fixture(SIZE, SIZE);
    let never = AtomicBool::new(false);
    let done = run(&f, 8, &never, recording_hooks(), slower);
    assert_eq!(done.result.expect("copy"), Some(SIZE));
    assert!(std::fs::read(&f.stage).expect("stage") == f.content);
    assert_eq!(done.verdicts, [false]);
    assert_eq!(done.progress.iter().sum::<u64>(), SIZE);
    // A drive that did not gain gets the rest the way it was copied before:
    // one stream through the page cache, never side-by-side ranges.
    assert_eq!(
        done.asked,
        [
            ("direct", 0, SECTION, 1),
            ("direct", SECTION, 2 * SECTION, 2),
            ("buffered", 2 * SECTION, SIZE, 1),
        ]
    );
    let calls = calls_of(f.writer.as_raw_fd());
    assert_eq!(threads_in(&calls, 2 * SECTION, SIZE), 1, "one thread");
    assert_eq!(calls.iter().map(|call| call.2 as u64).sum::<u64>(), SIZE);
}

/// Raised by the stand-in as soon as a copy reaches the second section.
static STOP: AtomicBool = AtomicBool::new(false);
static RAN_PAST_PROBE: AtomicUsize = AtomicUsize::new(0);

fn stop_in_second_section(
    reader: &File,
    writer: &File,
    from: u64,
    to: u64,
    length: usize,
) -> io::Result<usize> {
    if from >= 2 * SECTION {
        RAN_PAST_PROBE.fetch_add(1, Ordering::SeqCst);
    }
    if from >= SECTION {
        STOP.store(true, Ordering::SeqCst);
    }
    recording_call(reader, writer, from, to, length)
}

#[test]
fn speed_task_local_probe_cancel_in_a_section_gives_no_verdict_and_starts_no_rest() {
    let f = fixture(SIZE, SIZE);
    let mut cancelling = recording_hooks();
    cancelling.call = stop_in_second_section;
    let done = run(&f, 4, &STOP, cancelling, twice_as_fast);
    assert_eq!(done.result.expect("canceled"), None);
    assert!(done.verdicts.is_empty(), "an unfinished probe says nothing");
    assert_eq!(RAN_PAST_PROBE.load(Ordering::SeqCst), 0, "no rest copied");
}

#[test]
fn speed_task_local_probe_short_source_is_an_error_after_the_verdict() {
    // The source has the two sections but not the rest it was sized for.
    let f = fixture(2 * SECTION, SIZE);
    let never = AtomicBool::new(false);
    let done = run(&f, 4, &never, recording_hooks(), twice_as_fast);
    let error = done.result.expect_err("a shortened source fails");
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    assert_eq!(done.verdicts, [true], "the drive's verdict stands");
}

#[test]
fn speed_task_local_probe_real_file_is_identical_whatever_path_it_takes() {
    // The real system and clock on a file just long enough. Whether it goes
    // direct depends on the host (a quarter of the available memory, a block
    // device, O_DIRECT): then one verdict is reported, else the file is copied
    // plainly and none. The bytes are the same either way.
    let size = 128 * MIB;
    let f = fixture(size, 0);
    let mut writer = f.writer.try_clone().expect("stage handle");
    let never = AtomicBool::new(false);
    let verdicts = Mutex::new(Vec::new());
    let mut seen = 0u64;
    let copied = copy_probed(
        &f.reader,
        &mut writer,
        &never,
        &mut |bytes| seen += bytes,
        4,
        &|helps| verdicts.lock().expect("verdicts").push(helps),
    );
    assert_eq!(copied.expect("copy"), Some(size));
    assert_eq!(seen, size);
    assert!(verdicts.lock().expect("verdicts").len() <= 1);
    assert!(std::fs::read(&f.stage).expect("stage") == f.content);
}

#[test]
fn speed_task_local_probe_one_thread_or_a_short_file_copies_plainly() {
    let never = AtomicBool::new(false);
    for (size, threads) in [(128 * MIB, 1), (12 * MIB, 4)] {
        let f = fixture(size, 0);
        let mut writer = f.writer.try_clone().expect("stage handle");
        let verdicts = Mutex::new(Vec::new());
        let mut seen = 0u64;
        let copied = copy_probed(
            &f.reader,
            &mut writer,
            &never,
            &mut |bytes| seen += bytes,
            threads,
            &|helps| verdicts.lock().expect("verdicts").push(helps),
        );
        assert_eq!(copied.expect("copy"), Some(size), "{size} {threads}");
        assert_eq!(seen, size);
        assert!(verdicts.lock().expect("verdicts").is_empty(), "no probe");
        assert!(std::fs::read(&f.stage).expect("stage") == f.content);
    }
}
