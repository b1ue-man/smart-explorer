//! The hand-off under strain: long reads next to waiting prefetches, failed
//! prefetches, broken connections, a read that needs the prefetch's memory,
//! pastes that end quietly, the reservation before the buffer, and panics.
use super::catalog::TOO_LARGE_NOTE;
use super::data_object::Formats;
use super::handoff::{Config, Memory};
use super::test_fakes::FakeRemote;
use super::test_support::*;
use crate::transfer::ListedEntry;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{E_UNEXPECTED, STG_E_READFAULT};
use windows::Win32::System::Com::STREAM_SEEK_SET;

fn name(index: usize) -> String {
    format!("f{index}")
}

/// `count` files of `size` bytes; file n holds the byte n.
fn files(count: u8, size: usize) -> Vec<Vec<u8>> {
    (0..count).map(|index| vec![index; size]).collect()
}

fn remote_with(label: &str, files: &[Vec<u8>]) -> FakeRemote {
    files
        .iter()
        .enumerate()
        .fold(FakeRemote::new(label), |remote, (index, bytes)| {
            remote.file(&name(index), bytes.clone(), 0)
        })
}

#[test]
fn transfer_engine_task_remote_prefetch_survives_long_reads_and_idles_out() {
    let _apartment = Apartment::enter();
    let label = "transfer_engine_task long read";
    let files = files(6, 20_000);
    let remote = remote_with(label, &files);
    let stats = remote.stats.clone();
    let idle = Duration::from_millis(400);
    let config = Config {
        prefetch_idle: idle,
        ..Config::default()
    };
    let (object, _handoff) = in_process(remote, config);
    let formats = Formats::register();
    assert_eq!(descriptor(&object, &formats).unwrap().len(), 6);
    let first = contents(&object, &formats, 0).unwrap();
    wait_until("the next files are prefetched", || {
        (1..4).all(|index| stats.finished(&name(index)))
    });
    // Explorer spends three idle periods inside one file, reading all along.
    let mut read = Vec::new();
    let started = Instant::now();
    while started.elapsed() < idle * 3 {
        read.extend(read_exact(&first, 500));
        std::thread::sleep(idle / 8);
    }
    read.extend(read_all(&first).unwrap());
    assert_eq!(read, files[0]);
    drop(first);
    let second = contents(&object, &formats, 1).unwrap();
    assert_eq!(read_all(&second).unwrap(), files[1]);
    assert_eq!(stats.opens(&name(1)), 1, "reading kept the prefetched file");
    drop(second);
    // A quiet Explorer: the waiting prefetches give their memory back.
    std::thread::sleep(idle * 3);
    let fourth = contents(&object, &formats, 3).unwrap();
    assert_eq!(read_all(&fourth).unwrap(), files[3]);
    assert_eq!(
        stats.opens(&name(3)),
        2,
        "released while Explorer was quiet"
    );
}

#[test]
fn transfer_engine_task_remote_failed_prefetch_is_fetched_again() {
    let _apartment = Apartment::enter();
    let label = "transfer_engine_task failed prefetch";
    let second: Vec<u8> = (0..30_000u32).map(|index| (index % 241) as u8).collect();
    // The prefetch breaks at once; the fetch on request breaks mid-file and
    // resumes. Had the stream kept the failed prefetch, its one renewal would
    // be spent before the second break and Explorer would see an error.
    let remote = FakeRemote::new(label)
        .file("f0", vec![1; 10_000], 0)
        .failing_opens("f1", second.clone(), &[0, 12_000]);
    let stats = remote.stats.clone();
    let (object, _handoff) = in_process(remote, Config::default());
    let formats = Formats::register();
    descriptor(&object, &formats).unwrap();
    let first = contents(&object, &formats, 0).unwrap();
    assert_eq!(read_all(&first).unwrap(), vec![1; 10_000]);
    wait_until("the prefetch of f1 failed", || stats.failed("f1"));
    // Let the failed prefetch record its end before Explorer asks.
    std::thread::sleep(Duration::from_millis(100));
    let stream = contents(&object, &formats, 1).unwrap();
    assert_eq!(read_all(&stream).unwrap(), second);
    assert_eq!(stats.offsets("f1"), [0, 0, 12_000]);
    assert_eq!(external(label).errors, 0, "Explorer never saw a failure");
}

#[test]
fn transfer_engine_task_remote_broken_connection_resumes_at_position() {
    let _apartment = Apartment::enter();
    let formats = Formats::register();
    let data: Vec<u8> = (0..300_000u32).map(|index| (index % 253) as u8).collect();
    for seekable in [true, false] {
        let label = format!("transfer_engine_task resume {seekable}");
        let mut remote = FakeRemote::new(&label).failing_opens("big.bin", data.clone(), &[100_000]);
        if !seekable {
            remote = remote.not_seekable();
        }
        let stats = remote.stats.clone();
        let (object, _handoff) = in_process(remote, Config::default());
        descriptor(&object, &formats).unwrap();
        let stream = contents(&object, &formats, 0).unwrap();
        assert_eq!(read_all(&stream).unwrap(), data, "seekable: {seekable}");
        // Without a mid-file start the new connection reads past the bytes.
        let resumed_at = if seekable { 100_000 } else { 0 };
        assert_eq!(stats.offsets("big.bin"), [0, resumed_at]);
        assert_eq!(external(&label).errors, 0, "Explorer never saw the break");
    }
}

#[test]
fn transfer_engine_task_remote_waiting_read_takes_memory_from_prefetch() {
    let _apartment = Apartment::enter();
    let label = "transfer_engine_task yield memory";
    let files = files(4, 20_000);
    let remote = remote_with(label, &files);
    let stats = remote.stats.clone();
    // A fetch of 20 000 bytes holds 40 000 (buffer and read block): room for
    // two, and prefetching may use half.
    let budget = TestBudget::leaked(100_000);
    let config = Config {
        memory: Arc::new(TestMemory(budget)),
        ..Config::default()
    };
    let (object, _handoff) = in_process(remote, config);
    let formats = Formats::register();
    descriptor(&object, &formats).unwrap();
    let stream = contents(&object, &formats, 0).unwrap();
    assert_eq!(read_all(&stream).unwrap(), files[0]);
    wait_until("file 1 waits prefetched", || stats.finished(&name(1)));
    // Reading file 0 again (a rewound Clone) needs memory that only the
    // prefetched file 1 can give.
    let again = unsafe { stream.Clone() }.unwrap();
    unsafe { again.Seek(0, STREAM_SEEK_SET, None) }.unwrap();
    let started = Instant::now();
    assert_eq!(read_all(&again).unwrap(), files[0]);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the read waited for the prefetch instead of taking its memory"
    );
    drop((stream, again));
    let next = contents(&object, &formats, 1).unwrap();
    assert_eq!(read_all(&next).unwrap(), files[1]);
    assert_eq!(
        stats.opens(&name(1)),
        2,
        "the prefetch gave its memory away"
    );
    assert!(budget.max_used() <= 100_000);
}

#[test]
fn transfer_engine_task_remote_session_ends_complete_or_quiet() {
    let _apartment = Apartment::enter();
    let formats = Formats::register();
    // Every file arrived or failed, and no stream is open: the entry ends.
    let label = "transfer_engine_task session complete";
    let remote = FakeRemote::new(label)
        .file("a.txt", b"eins".to_vec(), 0)
        .failing("b.txt", vec![9; 100], 0);
    let (object, _handoff) = in_process(remote, Config::default());
    descriptor(&object, &formats).unwrap();
    let first = contents(&object, &formats, 0).unwrap();
    assert_eq!(read_all(&first).unwrap(), b"eins");
    let second = contents(&object, &formats, 1).unwrap();
    assert_eq!(read_all(&second).unwrap_err().code(), STG_E_READFAULT);
    assert!(!external(label).finished, "a stream is still open");
    drop((first, second));
    let entry = external(label);
    assert!(entry.finished, "arrived and failed files are all accounted");
    assert_eq!((entry.files_done, entry.errors), (1, 1));
    // Quiet with a file missing: the idle end finishes it as ended.
    let label = "transfer_engine_task session quiet";
    let remote = FakeRemote::new(label)
        .file("a.txt", b"eins".to_vec(), 0)
        .file("b.txt", b"zwei".to_vec(), 0);
    let (object, handoff) = in_process(remote, Config::default());
    descriptor(&object, &formats).unwrap();
    let stream = contents(&object, &formats, 0).unwrap();
    assert_eq!(read_all(&stream).unwrap(), b"eins");
    drop(stream);
    handoff.sessions.expire(Duration::from_secs(3_600));
    assert!(!external(label).finished, "not quiet long enough yet");
    handoff.sessions.expire(Duration::ZERO);
    let entry = external(label);
    assert!(entry.finished);
    assert_eq!(entry.note.as_deref(), Some("Explorer-Übergabe beendet"));
    // Windows could not take the list, so nothing is ever read: it ends too.
    let label = "transfer_engine_task session too large";
    let remote = FakeRemote::new(label).file("a.txt", b"eins".to_vec(), 0);
    let config = Config {
        alloc: refuse_alloc,
        ..Config::default()
    };
    let (object, handoff) = in_process(remote, config);
    assert!(descriptor(&object, &formats).is_err());
    handoff.sessions.expire(Duration::ZERO);
    let entry = external(label);
    assert!(entry.finished);
    assert_eq!(entry.note.as_deref(), Some(TOO_LARGE_NOTE));
}

#[test]
fn transfer_engine_task_remote_fetch_buffer_waits_for_its_reservation() {
    let _apartment = Apartment::enter();
    let label = "transfer_engine_task reservation first";
    let remote = FakeRemote::new(label).file("f", vec![5; 20_000], 0);
    let budget = TestBudget::leaked(50_000);
    let blocker = TestMemory(budget)
        .reserve(50_000, &AtomicBool::new(false))
        .expect("the whole budget");
    let config = Config {
        memory: Arc::new(TestMemory(budget)),
        ..Config::default()
    };
    let (_object, handoff) = in_process(remote, config);
    let entry = ListedEntry {
        rel: "f".to_string(),
        path: FakeRemote::path("f"),
        id: None,
        size: 20_000,
        size_known: true,
        mtime_ms: 0,
        is_dir: false,
    };
    let fetch = handoff.demand_fetch(&entry, 0).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(
        fetch.buffer_capacity(),
        0,
        "no buffer before its reservation"
    );
    drop(blocker);
    wait_until("the buffer after its reservation", || {
        fetch.buffer_capacity() >= 20_000
    });
}

#[test]
fn transfer_engine_task_remote_panics_end_as_errors() {
    let _apartment = Apartment::enter();
    let formats = Formats::register();
    let remote = FakeRemote::new("transfer_engine_task panicking listing")
        .file("a", vec![1], 0)
        .panicking_listing();
    let (object, _handoff) = in_process(remote, Config::default());
    let error = descriptor(&object, &formats).unwrap_err();
    assert_eq!(error.code(), E_UNEXPECTED, "an error instead of a hang");
    let remote = FakeRemote::new("transfer_engine_task panicking read").panicking("a", vec![1; 10]);
    let (object, _handoff) = in_process(remote, Config::default());
    descriptor(&object, &formats).unwrap();
    let stream = contents(&object, &formats, 0).unwrap();
    let error = read_all(&stream).unwrap_err();
    assert_eq!(error.code(), E_UNEXPECTED, "an error instead of a hang");
}
