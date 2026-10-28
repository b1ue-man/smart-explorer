//! Local packets (M3, 2026-10-28): one intent for many small files and what
//! a crash, "keep both", a race for a name and a cancel in the middle of a
//! packet leave behind. Crashes are simulated on the test thread: the packet
//! stops where the crash comes, the engine is dropped, and a resumed job
//! decides every file again (`missing_retry::resume`).
use super::super::super::roots::RootPlan;
use super::super::super::test_backend::{fwd, job, run as run_job, write};
use super::super::super::{job_store, side_flow, FolderRegister};
use super::*;
use crate::transfer::engine::view::JobView;
use crate::transfer::{Endpoint, RecoveryMode, TransferJob, TransferMsg};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Switches local packets off for every job (comparison benchmarks only;
/// they run alone with `--test-threads=1`).
pub(super) static PACKETS_OFF: AtomicBool = AtomicBool::new(false);
/// Target roots of the packets that ran, with their member counts.
static RAN: Mutex<Vec<(String, usize)>> = Mutex::new(Vec::new());

type After = Box<dyn FnMut(&Engine<'_>, usize)>;

thread_local! {
    static CRASH_BEFORE: Cell<Option<usize>> = const { Cell::new(None) };
    static ACKS_LOST: Cell<bool> = const { Cell::new(false) };
    static AFTER: RefCell<Option<After>> = const { RefCell::new(None) };
}

pub(super) fn ran(root: &str, members: usize) {
    RAN.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push((root.to_string(), members));
}

fn packets_into(root: &Path) -> Vec<usize> {
    let root = fwd(root);
    RAN.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .filter(|(at, _)| *at == root)
        .map(|(_, members)| *members)
        .collect()
}

/// The crash comes before member `index` (0: right after the intent).
pub(super) fn crash_before(index: usize) -> bool {
    CRASH_BEFORE.with(Cell::get) == Some(index)
}

/// The crash comes after a publication, before its acknowledgement.
pub(super) fn ack_lost() -> bool {
    ACKS_LOST.with(Cell::get)
}

pub(super) fn after(engine: &Engine<'_>, index: usize) {
    AFTER.with(|after| {
        if let Some(after) = after.borrow_mut().as_mut() {
            after(engine, index);
        }
    });
}

/// Clears this thread's hooks when a test ends.
struct Hooks;

impl Drop for Hooks {
    fn drop(&mut self) {
        CRASH_BEFORE.with(|crash| crash.set(None));
        ACKS_LOST.with(|lost| lost.set(false));
        AFTER.with(|after| after.borrow_mut().take());
    }
}

fn open<'a>(
    candidate: &'a TransferJob,
    cancel: &'a AtomicBool,
    report: &'a (dyn Fn(TransferMsg) + Sync),
) -> Engine<'a> {
    let view = JobView::of(candidate);
    let record = job_store::begin(&view, view.target_dir, cancel).unwrap();
    let folders = FolderRegister::new(
        view.target,
        view.target_dir,
        side_flow(view.target, view.target_dir),
    );
    let mut engine = Engine::new(
        view,
        view.kind(),
        folders,
        cancel,
        report,
        &RootPlan::default(),
    );
    engine.job_record = record;
    engine
}

fn work(path: &Path, rel: &str) -> FileWork {
    let meta = std::fs::metadata(path).unwrap();
    FileWork::new(
        fwd(path),
        rel.into(),
        meta.len(),
        crate::local_access::system_time_ms(meta.modified().unwrap()),
    )
}

fn names(path: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Private local stages (`.smart-explorer-….part`) in `path`.
fn is_stage(name: &str) -> bool {
    name.contains(".smart-explorer-") && name.ends_with(".part")
}

/// The published names in `path` (stages left out) and the stage count.
fn published(path: &Path) -> (Vec<String>, usize) {
    let (stages, published): (Vec<String>, Vec<String>) =
        names(path).into_iter().partition(|name| is_stage(name));
    (published, stages.len())
}

/// `count` small sources `f<i>.txt` (content `file <i>`) and an empty
/// target folder.
fn layout(root: &Path, count: usize) -> (Vec<PathBuf>, PathBuf) {
    let sources: Vec<PathBuf> = (0..count)
        .map(|index| {
            let path = root.join(format!("src/f{index}.txt"));
            write(&path, format!("file {index}").as_bytes());
            path
        })
        .collect();
    let target = root.join("target");
    std::fs::create_dir(&target).unwrap();
    (sources, target)
}

fn works(sources: &[PathBuf]) -> Vec<FileWork> {
    sources
        .iter()
        .enumerate()
        .map(|(index, source)| work(source, &format!("f{index}.txt")))
        .collect()
}

fn packet(engine: &Engine<'_>, files: Vec<FileWork>) {
    run(
        engine,
        files,
        &mut [0; super::super::super::ops::COPY_BUFFER],
    );
}

fn apply(engine: &Engine<'_>, file: FileWork) {
    run_file(
        engine,
        file,
        &mut [0; super::super::super::ops::COPY_BUFFER],
    );
}

/// Every source once at its own name, with its own content; nothing else.
fn exactly_once(target: &Path, count: usize) {
    let expected: Vec<String> = {
        let mut expected: Vec<String> = (0..count).map(|i| format!("f{i}.txt")).collect();
        expected.sort();
        expected
    };
    assert_eq!(names(target), expected);
    for index in 0..count {
        let path = target.join(format!("f{index}.txt"));
        let content = std::fs::read(path).unwrap();
        assert_eq!(content, format!("file {index}").as_bytes());
    }
}

#[test]
fn speed_task_local_packet_takes_its_fair_share_of_the_queue() {
    assert_eq!(take_beyond_first(0, 8), None, "one file: alone");
    assert_eq!(take_beyond_first(14, 8), None, "below two per permit");
    assert_eq!(take_beyond_first(15, 8), Some(1), "two per permit");
    assert_eq!(take_beyond_first(159, 16), Some(9), "ten per permit");
    assert_eq!(take_beyond_first(100_000, 16), Some(MAX_FILES - 1));
    assert_eq!(take_beyond_first(3, 0), Some(3), "a limit of 0 counts as 1");
}

#[test]
fn speed_task_local_packet_crash_after_the_intent_publishes_nothing_twice() {
    let _hooks = Hooks;
    let root = tempfile::tempdir().unwrap();
    let (sources, target) = layout(root.path(), 6);
    let selected: Vec<&Path> = sources.iter().map(PathBuf::as_path).collect();
    let mut candidate = job(Endpoint::Local, Endpoint::Local, &target, &selected);
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    {
        let engine = open(&candidate, &cancel, &report);
        engine.job_record.as_ref().unwrap().prepare_apply().unwrap();
        CRASH_BEFORE.with(|crash| crash.set(Some(0)));
        packet(&engine, works(&sources));
        // One durable intent for every member, written after their stages
        // (R4): nothing is published, every stage is recorded.
        for file in works(&sources) {
            assert!(file_recovery::has_receipt(&engine, &file), "{}", file.rel);
        }
        assert_eq!(published(&target), (Vec::<String>::new(), 6));
    }
    CRASH_BEFORE.with(|crash| crash.set(None));
    candidate.recovery = RecoveryMode::Resume;
    let engine = open(&candidate, &cancel, &report);
    for file in works(&sources) {
        apply(&engine, file);
    }
    assert_eq!(engine.issues.total(), 0, "{:?}", engine.issues.shown().0);
    // The resumed job removed every recorded stage: no orphan is left.
    exactly_once(&target, 6);
}

#[test]
fn speed_task_local_packet_crash_before_acknowledgements_loses_nothing() {
    let _hooks = Hooks;
    let root = tempfile::tempdir().unwrap();
    let (sources, target) = layout(root.path(), 5);
    let selected: Vec<&Path> = sources.iter().map(PathBuf::as_path).collect();
    let mut candidate = job(Endpoint::Local, Endpoint::Local, &target, &selected);
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    {
        let engine = open(&candidate, &cancel, &report);
        engine.job_record.as_ref().unwrap().prepare_apply().unwrap();
        // Two members are published, neither acknowledged; then the crash.
        ACKS_LOST.with(|lost| lost.set(true));
        CRASH_BEFORE.with(|crash| crash.set(Some(2)));
        packet(&engine, works(&sources));
        // The three not yet published are staged and recorded.
        let expected = vec!["f0.txt".to_string(), "f1.txt".to_string()];
        assert_eq!(published(&target), (expected, 3));
    }
    ACKS_LOST.with(|lost| lost.set(false));
    CRASH_BEFORE.with(|crash| crash.set(None));
    candidate.recovery = RecoveryMode::Resume;
    let engine = open(&candidate, &cancel, &report);
    for file in works(&sources) {
        apply(&engine, file);
    }
    assert_eq!(engine.issues.total(), 0, "{:?}", engine.issues.shown().0);
    // The two published files are this job's own (no numbered second copy);
    // the three staged ones arrive once and none of their stages is left.
    exactly_once(&target, 5);
}

#[test]
fn speed_task_local_packet_keep_both_numbers_inside_the_packet() {
    let _hooks = Hooks;
    let root = tempfile::tempdir().unwrap();
    let (sources, target) = layout(root.path(), 5);
    // Foreign files at two requested names before the packet, and at a
    // third one after the intent (a race for the name).
    write(&target.join("f1.txt"), b"foreign one");
    write(&target.join("f3.txt"), b"foreign three");
    let raced = target.join("f4.txt");
    AFTER.with(|after| {
        *after.borrow_mut() = Some(Box::new(move |_: &Engine<'_>, index: usize| {
            if index == 0 {
                write(&raced, b"foreign four");
            }
        }));
    });
    let selected: Vec<&Path> = sources.iter().map(PathBuf::as_path).collect();
    let candidate = job(Endpoint::Local, Endpoint::Local, &target, &selected);
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    let engine = open(&candidate, &cancel, &report);
    engine.job_record.as_ref().unwrap().prepare_apply().unwrap();
    packet(&engine, works(&sources));
    assert_eq!(engine.issues.total(), 0, "{:?}", engine.issues.shown().0);
    assert_eq!(packets_into(&target), [5]);
    assert_eq!(
        names(&target),
        [
            "f0.txt",
            "f1 (2).txt",
            "f1.txt",
            "f2.txt",
            "f3 (2).txt",
            "f3.txt",
            "f4 (2).txt",
            "f4.txt",
        ]
    );
    let read = |name: &str| std::fs::read(target.join(name)).unwrap();
    assert_eq!(read("f1.txt"), b"foreign one", "never replaced");
    assert_eq!(read("f3.txt"), b"foreign three");
    assert_eq!(read("f4.txt"), b"foreign four");
    assert_eq!(read("f1 (2).txt"), b"file 1");
    assert_eq!(read("f3 (2).txt"), b"file 3");
    assert_eq!(read("f4 (2).txt"), b"file 4");
    assert_eq!(read("f2.txt"), b"file 2");
}

#[test]
fn speed_task_local_packet_cancel_mid_packet_publishes_no_more_and_resumes() {
    let _hooks = Hooks;
    let root = tempfile::tempdir().unwrap();
    let (sources, target) = layout(root.path(), 8);
    AFTER.with(|after| {
        *after.borrow_mut() = Some(Box::new(|engine: &Engine<'_>, index: usize| {
            if index == 2 {
                engine.halt();
            }
        }));
    });
    let selected: Vec<&Path> = sources.iter().map(PathBuf::as_path).collect();
    let mut candidate = job(Endpoint::Local, Endpoint::Local, &target, &selected);
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    {
        let engine = open(&candidate, &cancel, &report);
        engine.job_record.as_ref().unwrap().prepare_apply().unwrap();
        packet(&engine, works(&sources));
        assert_eq!(engine.issues.total(), 0, "{:?}", engine.issues.shown().0);
        // Three published, nothing after the stop, no stage left behind.
        assert_eq!(names(&target), ["f0.txt", "f1.txt", "f2.txt"]);
    }
    AFTER.with(|after| after.borrow_mut().take());
    candidate.recovery = RecoveryMode::Resume;
    let engine = open(&candidate, &cancel, &report);
    for file in works(&sources) {
        apply(&engine, file);
    }
    assert_eq!(engine.issues.total(), 0, "{:?}", engine.issues.shown().0);
    exactly_once(&target, 8);
}

/// A whole job of many small files: packets form, every file arrives once.
#[test]
fn speed_task_local_packet_job_packs_small_files() {
    let root = tempfile::tempdir().unwrap();
    // Enough files that every flow limit (at most 256) leaves two per permit.
    let (sources, target) = layout(root.path(), 1200);
    let folder = root.path().join("src");
    let copy = job(Endpoint::Local, Endpoint::Local, &target, &[&folder]);
    let finished = run_job(copy);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, sources.len() as u64);
    let copied = target.join("src");
    exactly_once(&copied, 1200);
    let packets = packets_into(&target);
    assert!(!packets.is_empty(), "packets formed");
    let sizes = 2..=MAX_FILES;
    assert!(packets.iter().all(|members| sizes.contains(members)));
}
