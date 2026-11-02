use super::*;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;

const MIB: u64 = 1024 * 1024;

struct Fixture {
    _dir: tempfile::TempDir,
    reader: File,
    writer: File,
    stage: PathBuf,
    content: Vec<u8>,
}

fn bytes(length: u64) -> Vec<u8> {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    (0..length)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 32) as u8
        })
        .collect()
}

/// A source with `content` and a stage already sized to `stage_len` (sparse,
/// as the caller of `copy_ranges` prepares it).
fn fixture_with(content: Vec<u8>, stage_len: u64) -> Fixture {
    let dir = tempfile::tempdir().expect("temp dir");
    let source = dir.path().join("source.bin");
    File::create(&source)
        .and_then(|mut file| file.write_all(&content))
        .expect("source");
    let stage = dir.path().join("stage.bin");
    let writer = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&stage)
        .expect("stage");
    writer.set_len(stage_len).expect("sparse stage");
    Fixture {
        reader: File::open(&source).expect("reader"),
        writer,
        stage,
        content,
        _dir: dir,
    }
}

fn fixture(length: u64, stage_len: u64) -> Fixture {
    fixture_with(bytes(length), stage_len)
}

impl Fixture {
    fn copy(&self, size: u64, threads: usize, hooks: &Hooks) -> io::Result<Option<u64>> {
        let cancel = AtomicBool::new(false);
        copy_ranges_with(
            &self.reader,
            &self.writer,
            size,
            threads,
            &cancel,
            &mut |_| {},
            hooks,
        )
    }

    fn stage_is_source(&self) -> bool {
        std::fs::read(&self.stage).expect("stage") == self.content
    }
}

#[test]
fn speed_task_local_ranges_copy_is_identical_to_single_thread() {
    let size = 10 * MIB;
    let ranged = fixture(size, size);
    assert_eq!(
        ranged.copy(size, 4, &Hooks::SYSTEM).expect("ranges"),
        Some(size)
    );
    assert!(
        ranged.stage_is_source(),
        "four ranges differ from the source"
    );
    // The single-thread path of the same module produces the same bytes.
    let mut single = fixture_with(ranged.content.clone(), 0);
    let never = AtomicBool::new(false);
    let copied =
        super::super::copy_handles(&single.reader, &mut single.writer, &never, &mut |_| {}, 1);
    assert_eq!(copied.expect("single"), Some(size));
    assert!(single.stage_is_source());
}

/// Refuses every call the way another file system does.
static REFUSED_CALLS: AtomicUsize = AtomicUsize::new(0);

fn refuse(_: &File, _: &File, _: u64, _: u64, _: usize) -> io::Result<usize> {
    REFUSED_CALLS.fetch_add(1, Ordering::SeqCst);
    Err(io::Error::from_raw_os_error(libc::EXDEV))
}

/// The first call works (moves nothing), the next ones are refused.
static MIXED_CALLS: AtomicUsize = AtomicUsize::new(0);

fn work_then_exdev(_: &File, _: &File, _: u64, _: u64, length: usize) -> io::Result<usize> {
    match MIXED_CALLS.fetch_add(1, Ordering::SeqCst) {
        0 => Ok(length),
        _ => Err(io::Error::from_raw_os_error(libc::EXDEV)),
    }
}

#[test]
fn speed_task_local_ranges_fall_back_to_pread_pwrite_when_copy_file_range_is_refused() {
    let size = 10 * MIB;
    let f = fixture(size, size);
    let hooks = Hooks {
        call: refuse,
        may_spawn: always,
    };
    assert_eq!(f.copy(size, 4, &hooks).expect("ranges"), Some(size));
    assert!(f.stage_is_source());
    // Each range tried the kernel once and then stayed with pread/pwrite.
    assert_eq!(REFUSED_CALLS.load(Ordering::SeqCst), 4);
    for code in [
        libc::ENOSYS,
        libc::EXDEV,
        libc::EINVAL,
        libc::EOPNOTSUPP,
        libc::EBADF,
    ] {
        assert!(is_refusal(&io::Error::from_raw_os_error(code)), "{code}");
    }
    assert!(!is_refusal(&io::Error::from_raw_os_error(libc::EIO)));
    // Only the FIRST call of a range may be refused: a later refusal is a
    // real failure and is reported, not papered over.
    let f = fixture(20 * MIB, 20 * MIB);
    let hooks = Hooks {
        call: work_then_exdev,
        may_spawn: always,
    };
    let failed = f.copy(20 * MIB, 1, &hooks).expect_err("second call fails");
    assert_eq!(failed.raw_os_error(), Some(libc::EXDEV));
}

/// Takes its time and moves nothing: the call count shows how far it got.
static SLOW_CALLS: AtomicUsize = AtomicUsize::new(0);

fn slow(_: &File, _: &File, _: u64, _: u64, length: usize) -> io::Result<usize> {
    SLOW_CALLS.fetch_add(1, Ordering::SeqCst);
    std::thread::sleep(std::time::Duration::from_millis(10));
    Ok(length)
}

#[test]
fn speed_task_local_ranges_cancel_stops_all_threads() {
    // 512 MiB in four ranges of sixteen calls each; nothing is written.
    let f = fixture(MIB, 0);
    let cancel = AtomicBool::new(false);
    let hooks = Hooks {
        call: slow,
        may_spawn: always,
    };
    let mut cancel_on_progress = |_: u64| cancel.store(true, Ordering::Release);
    let result = copy_ranges_with(
        &f.reader,
        &f.writer,
        512 * MIB,
        4,
        &cancel,
        &mut cancel_on_progress,
        &hooks,
    );
    assert!(matches!(result, Ok(None)), "canceled: {result:?}");
    let calls = SLOW_CALLS.load(Ordering::SeqCst);
    assert!(
        calls > 0 && calls < 64,
        "stopped early: {calls} of 64 calls"
    );
}

#[test]
fn speed_task_local_ranges_progress_sums_to_size() {
    let size = 9 * MIB + 123;
    let f = fixture(size, size);
    let caller = std::thread::current().id();
    let (mut total, mut reports) = (0u64, 0usize);
    let cancel = AtomicBool::new(false);
    let mut progress = |bytes: u64| {
        assert_eq!(
            std::thread::current().id(),
            caller,
            "only the caller reports"
        );
        total += bytes;
        reports += 1;
    };
    let result = copy_ranges(&f.reader, &f.writer, size, 3, &cancel, &mut progress);
    assert_eq!(result.expect("ranges"), Some(size));
    assert_eq!(total, size);
    assert!(reports >= 1 && f.stage_is_source());
}

#[test]
fn speed_task_local_ranges_short_source_is_unexpected_eof() {
    // Announced with 12 MiB, but only 10 are there (shortened meanwhile).
    let f = fixture(10 * MIB, 12 * MIB);
    let error = f
        .copy(12 * MIB, 2, &Hooks::SYSTEM)
        .expect_err("short source");
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn speed_task_local_ranges_thread_refusal_degrades_to_fewer_threads() {
    let size = 10 * MIB + 5;
    // Only the first range gets a thread; the caller copies the rest.
    let f = fixture(size, size);
    let hooks = Hooks {
        may_spawn: |index| index == 0,
        ..Hooks::SYSTEM
    };
    assert_eq!(f.copy(size, 4, &hooks).expect("ranges"), Some(size));
    assert!(f.stage_is_source());
    // No thread at all: the calling thread does everything, without a panic.
    let f = fixture(size, size);
    let hooks = Hooks {
        may_spawn: |_| false,
        ..Hooks::SYSTEM
    };
    assert_eq!(f.copy(size, 4, &hooks).expect("ranges"), Some(size));
    assert!(f.stage_is_source());
}

#[test]
fn speed_task_local_ranges_split_borders_on_whole_mebibytes() {
    let mib = |from: u64, to: u64| (from * MIB, to * MIB);
    assert_eq!(
        split(10 * MIB, 4),
        [mib(0, 2), mib(2, 4), mib(4, 6), mib(6, 10)]
    );
    assert_eq!(split(3 * MIB + 7, 8).len(), 3, "a mebibyte each at least");
    assert_eq!(split(100, 4), [(0, 100)], "never less than one range");
}
