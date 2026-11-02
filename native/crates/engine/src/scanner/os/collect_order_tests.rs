//! Read-ahead listings keep the order, the messages, the budgets and the access
//! requests of a copy expansion. Each test compares the walk that lists every
//! folder itself (0 listing threads) with walks that read ahead. Test names start
//! with `abgleich_task_`.
use super::listing;
use super::{collect, CollectOutcome, Limits, ReadGrant};
use crate::source_media::{test_profiles, Source};
use std::cell::RefCell;
use std::io::ErrorKind;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A temporary folder tree, removed when dropped.
struct Tree(PathBuf);

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn put(root: &Path, relative: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("a file has a parent")).expect("folder");
    std::fs::write(path, b"x").expect("file");
}

/// `a/{x1, x2, sub/deep/{y, z}}`, `b/{b1, b2, b3}`, the empty `c`, `d/d00..d19/f`
/// and `top`: 54 entries, folders included.
fn fixture(label: &str) -> Tree {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let root = std::env::temp_dir().join(format!(
        "se_collect_order_{label}_{}_{nanos}",
        std::process::id()
    ));
    let tree = Tree(root.clone());
    for relative in [
        "a/x1.txt",
        "a/x2.txt",
        "a/sub/deep/y.txt",
        "a/sub/deep/z.txt",
        "b/b1.txt",
        "b/b2.txt",
        "b/b3.txt",
        "top.txt",
    ] {
        put(&root, relative);
    }
    std::fs::create_dir_all(root.join("c")).expect("empty folder");
    for number in 0..20 {
        put(&root, &format!("d/d{number:02}/f.txt"));
    }
    tree
}

/// Opens every folder below `dir` once, as a walk does. On NTFS a folder's
/// times in its parent's listing (what the walk reports) catch up with the
/// files created in it only once the folder itself is opened again; without
/// this the first walk of a fresh tree can report older folder times than
/// the walks after it.
fn settle(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            settle(&entry.path());
        }
    }
}

/// The walk of `root` with the given listing threads, budgets and grant.
fn walk_with(
    root: &Path,
    threads: usize,
    limits: Limits,
    grant: Option<&dyn ReadGrant>,
) -> CollectOutcome {
    settle(root);
    let cancel = AtomicBool::new(false);
    collect(root, false, 1, &cancel, grant, limits, threads)
}

/// Everything a caller can see of an outcome, in the order it is reported.
type Seen = (Vec<String>, Vec<(String, String)>, u64, bool, u64, bool);

fn seen(outcome: &CollectOutcome) -> Seen {
    (
        outcome
            .entries
            .iter()
            .map(|entry| format!("{entry:?}"))
            .collect(),
        outcome
            .issues
            .iter()
            .map(|issue| (issue.path.clone(), issue.detail.clone()))
            .collect(),
        outcome.suppressed_issues,
        outcome.canceled,
        outcome.omitted,
        outcome.is_complete(),
    )
}

fn injected() -> MutexGuard<'static, Vec<(PathBuf, ErrorKind)>> {
    listing::INJECTED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

fn lift(folder: &Path) {
    injected().retain(|(path, _)| path != folder);
}

/// Makes opening `folder` fail with `kind` until the guard drops.
struct Denied(PathBuf);

impl Denied {
    fn new(folder: &Path, kind: ErrorKind) -> Self {
        injected().push((folder.to_path_buf(), kind));
        Self(folder.to_path_buf())
    }
}

impl Drop for Denied {
    fn drop(&mut self) {
        lift(&self.0);
    }
}

/// A read grant that records its requests. An accepting answer lifts the folders
/// in `lifts`.
struct Grant {
    answer: Result<bool, String>,
    lifts: Vec<PathBuf>,
    requests: RefCell<Vec<String>>,
}

impl ReadGrant for Grant {
    fn can_request(&self, _requested: &str) -> bool {
        true
    }

    fn request(&self, requested: &str) -> Result<bool, String> {
        self.requests.borrow_mut().push(requested.to_string());
        if matches!(self.answer, Ok(true)) {
            for folder in &self.lifts {
                lift(folder);
            }
        }
        self.answer.clone()
    }
}

/// One walk of a fixture whose `b` and `d/d05` cannot be opened until the grant
/// lifts them. Returns what the caller sees and the requests the walk made.
fn access_walk(root: &Path, threads: usize, answer: Result<bool, String>) -> (Seen, Vec<String>) {
    let blocked = [root.join("b"), root.join("d").join("d05")];
    let _denied: Vec<Denied> = blocked
        .iter()
        .map(|folder| Denied::new(folder, ErrorKind::PermissionDenied))
        .collect();
    let grant = Grant {
        answer,
        lifts: blocked.to_vec(),
        requests: RefCell::new(Vec::new()),
    };
    let outcome = walk_with(root, threads, Limits::PRODUCTION, Some(&grant));
    (seen(&outcome), grant.requests.into_inner())
}

#[test]
fn abgleich_task_listing_threads_keep_the_serial_order_and_entries() {
    let tree = fixture("order");
    let serial = seen(&walk_with(&tree.0, 0, Limits::PRODUCTION, None));
    assert!(serial.5, "the fixture is walked completely");
    assert!(serial.0.len() > 40, "the fixture has its folders and files");
    for threads in [1, 2, 4, 16] {
        assert_eq!(
            seen(&walk_with(&tree.0, threads, Limits::PRODUCTION, None)),
            serial,
            "{threads} listing thread(s) keep the serial order"
        );
    }
}

#[test]
fn abgleich_task_listing_threads_report_the_same_failures_in_the_same_order() {
    let tree = fixture("failures");
    let _denied = [
        Denied::new(&tree.0.join("a").join("sub"), ErrorKind::PermissionDenied),
        Denied::new(&tree.0.join("d").join("d05"), ErrorKind::Other),
    ];
    let serial = seen(&walk_with(&tree.0, 0, Limits::PRODUCTION, None));
    assert_eq!(serial.1.len(), 2, "both failing folders are reported");
    assert!(
        !serial.5,
        "a failing folder makes the copy expansion incomplete"
    );
    for threads in [1, 2, 4] {
        assert_eq!(
            seen(&walk_with(&tree.0, threads, Limits::PRODUCTION, None)),
            serial,
            "{threads} listing thread(s) report the same failures"
        );
    }
}

#[test]
fn abgleich_task_listing_threads_keep_the_budgets_and_their_messages() {
    let tree = fixture("budgets");
    let _denied = [
        Denied::new(&tree.0.join("a").join("sub"), ErrorKind::PermissionDenied),
        Denied::new(&tree.0.join("d").join("d05"), ErrorKind::Other),
    ];
    let limits = [
        Limits {
            entries: 7,
            ..Limits::PRODUCTION
        },
        Limits {
            name_bytes: 400,
            ..Limits::PRODUCTION
        },
        Limits {
            depth: 2,
            ..Limits::PRODUCTION
        },
        Limits {
            issues: 1,
            ..Limits::PRODUCTION
        },
    ];
    for limit in limits {
        let serial = seen(&walk_with(&tree.0, 0, limit, None));
        assert_eq!(
            seen(&walk_with(&tree.0, 4, limit, None)),
            serial,
            "{limit:?} stops at the same entry with the same message"
        );
    }
}

#[test]
fn abgleich_task_listing_threads_ask_for_access_once_at_the_same_folder() {
    let tree = fixture("access");
    for answer in [Ok(true), Ok(false), Err("grant refused".to_string())] {
        let serial = access_walk(&tree.0, 0, answer.clone());
        let parallel = access_walk(&tree.0, 4, answer.clone());
        assert_eq!(parallel, serial, "answer {answer:?}");
        assert_eq!(
            serial.1.len(),
            1,
            "one request, made at the first failing folder in walk order"
        );
    }
}

#[test]
fn abgleich_task_listing_threads_keep_the_protected_device_guard_exclusive() {
    let tree = fixture("guard");
    let device = format!("abgleich-task-listing-{}", tree.0.display());
    let _profile = test_profiles::register(
        &tree.0.join("a"),
        Source {
            device: device.clone(),
            generation: "abgleich-task".to_string(),
            read_only: true,
        },
    );
    let serial = seen(&walk_with(&tree.0, 0, Limits::PRODUCTION, None));
    assert_eq!(
        seen(&walk_with(&tree.0, 4, Limits::PRODUCTION, None)),
        serial,
        "the protected folders are listed as the serial walk lists them"
    );
    assert!(serial.5, "the copy expansion is complete");
    assert!(
        test_profiles::acquisitions(&device) >= 2,
        "the protected folders took the device lock"
    );
    assert_eq!(
        test_profiles::maximum(&device),
        1,
        "one reader at a time on the protected device"
    );
}

#[test]
fn abgleich_task_listing_threads_read_a_folder_above_the_read_ahead_cap_at_its_turn() {
    let tree = fixture("cap");
    let wide = tree.0.join("wide");
    std::fs::create_dir_all(&wide).expect("wide folder");
    for number in 0..listing::MAX_AHEAD_ENTRIES + 3 {
        std::fs::write(wide.join(format!("f{number:05}.txt")), b"x").expect("file");
    }
    let serial = seen(&walk_with(&tree.0, 0, Limits::PRODUCTION, None));
    assert!(serial.5, "the copy expansion is complete");
    assert!(
        serial.0.len() > listing::MAX_AHEAD_ENTRIES,
        "the folder above the cap is walked"
    );
    assert_eq!(
        seen(&walk_with(&tree.0, 4, Limits::PRODUCTION, None)),
        serial,
        "the folder above the cap is read by the walk at its turn"
    );
}

#[test]
fn abgleich_task_listing_threads_finish_a_wide_tree() {
    let tree = fixture("wide");
    for number in 0..300 {
        let folder = tree.0.join(format!("many/m{number:03}"));
        std::fs::create_dir_all(&folder).expect("folder");
        std::fs::write(folder.join("x.txt"), b"x").expect("file");
    }
    let root = tree.0.clone();
    let (sent, finished) = mpsc::channel();
    std::thread::spawn(move || {
        let cancel = AtomicBool::new(false);
        let outcome = collect(&root, false, 1, &cancel, None, Limits::PRODUCTION, 8);
        let _ = sent.send(outcome.is_complete() && outcome.entries.len() > 600);
    });
    assert_eq!(
        finished.recv_timeout(Duration::from_secs(120)),
        Ok(true),
        "the walk with 8 listing threads ends with a complete result"
    );
}

#[test]
fn abgleich_task_listing_threads_follow_the_cores_available_at_run_time() {
    let available = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
    assert_eq!(listing::threads(), available.min(16));
}

#[test]
fn abgleich_task_listing_threads_honor_a_cancel_before_the_walk() {
    let tree = fixture("cancel");
    let cancel = AtomicBool::new(true);
    let outcome = collect(&tree.0, false, 1, &cancel, None, Limits::PRODUCTION, 4);
    assert!(outcome.canceled, "the cancel is reported");
    assert!(
        outcome.entries.is_empty(),
        "no entry is listed after the cancel"
    );
}

/// Waits until a foreground reader waits for the device lock of `device`.
fn wait_for_a_reader_on(device: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !crate::source_media::snapshots()
        .iter()
        .any(|status| status.source.device == device && status.foreground_waiters > 0)
    {
        assert!(
            std::time::Instant::now() < deadline,
            "no reader waits for the device lock of {device}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn abgleich_task_listing_threads_end_a_wait_for_a_device_lock_when_the_walk_ends() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let root = Tree(std::env::temp_dir().join(format!(
        "se_collect_order_lock_{}_{nanos}",
        std::process::id()
    )));
    std::fs::create_dir_all(root.0.join("alpha")).expect("folder");
    std::fs::create_dir_all(root.0.join("beta")).expect("folder");
    // The walk pushes the two folders in listing order and takes the last one
    // first. `late` is taken first; `early` is not taken before the walk ends, so
    // its listing (started by a listing thread) still waits for its device then.
    let listed: Vec<std::ffi::OsString> = crate::local_access::read_directory(&root.0)
        .expect("root listing")
        .map(|entry| entry.expect("entry").name)
        .collect();
    assert_eq!(listed.len(), 2, "the root holds the two folders");
    let early = root.0.join(&listed[0]);
    let late = root.0.join(&listed[1]);
    for number in 0..3 {
        std::fs::write(late.join(format!("f{number}.txt")), b"x").expect("file");
    }
    let device_early = format!("abgleich-task-lock-early-{}", root.0.display());
    let device_late = format!("abgleich-task-lock-late-{}", root.0.display());
    let source = |device: &str| Source {
        device: device.to_string(),
        generation: "abgleich-task".to_string(),
        read_only: true,
    };
    let _profile_early = test_profiles::register(&early, source(device_early.as_str()));
    let _profile_late = test_profiles::register(&late, source(device_late.as_str()));
    let held_early = crate::source_media::acquire_source(
        &early,
        source(device_early.as_str()),
        crate::source_media::Priority::Foreground,
        || false,
    )
    .expect("the early device lock");
    let held_late = crate::source_media::acquire_source(
        &late,
        source(device_late.as_str()),
        crate::source_media::Priority::Foreground,
        || false,
    )
    .expect("the late device lock");
    let (sent, finished) = mpsc::channel();
    let walk_root = root.0.clone();
    std::thread::spawn(move || {
        let cancel = AtomicBool::new(false);
        let limits = Limits {
            entries: 3,
            ..Limits::PRODUCTION
        };
        let outcome = collect(&walk_root, false, 1, &cancel, None, limits, 2);
        let _ = sent.send(seen(&outcome));
    });
    // The walk waits for `late` while its device is held. Its entry budget is
    // reached inside `late`, with `early` still waiting for its device; the walk
    // must end then, and its caller's cancel flag stays unset. `late`'s device is
    // released only after that wait is observed, so the walk cannot end earlier.
    wait_for_a_reader_on(device_early.as_str());
    drop(held_late);
    let walk = finished
        .recv_timeout(Duration::from_secs(20))
        .expect("the walk ends while the early device is still held");
    drop(held_early);
    assert_eq!(walk.0.len(), 3, "the walk stops at its entry budget");
    assert_eq!(walk.1.len(), 1, "the budget is reported once");
    assert!(!walk.3, "the caller's cancel flag is not set");
    assert!(!walk.5, "the walk is incomplete");
}
