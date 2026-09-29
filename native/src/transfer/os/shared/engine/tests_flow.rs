//! Concurrency and bookkeeping of the engine: parallel workers, deep trees at
//! the smallest limits (K2), two jobs sharing one connection, a refused read
//! access, the issue log and jobs started through the lane.
use super::test_backend::{fwd, job, run, unique, write, Fake, Finished};
use super::{Engine, FolderRegister, JobView, Side};
use crate::transfer::{Endpoint, JobItems, Layout, TransferKind, TransferMsg, TransferRequest};
use crate::types::{Conflict, CopyMode};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

/// Runs a job on its own thread; a job that does not end in time failed
/// (a deadlock would otherwise hang the suite).
fn run_within(transfer: crate::transfer::TransferJob, limit: Duration) -> Finished {
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done_tx.send(run(transfer));
    });
    done_rx
        .recv_timeout(limit)
        .expect("the job finished instead of waiting forever")
}

fn deep_tree(root: &Path, depth: usize) -> PathBuf {
    let top = root.join("d0");
    let mut folder = top.clone();
    for level in 0..depth {
        write(
            &folder.join(format!("f{level}.txt")),
            format!("level {level}").as_bytes(),
        );
        folder = folder.join(format!("d{}", level + 1));
    }
    top
}

#[test]
fn transfer_engine_task_workers_run_in_parallel() {
    let root = tempfile::tempdir().expect("temp dir");
    let remote = root.path().join("remote/Vault");
    for index in 0..12 {
        write(
            &remote.join(format!("f{index:02}.bin")),
            &vec![1u8; 600 * 1024],
        );
    }
    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");
    let mut fake = Fake::new("parallel");
    fake.read_delay = Duration::from_millis(40);
    let (fake, handle) = fake.handle();
    let finished = run_within(
        job(Endpoint::Remote(handle), Endpoint::Local, &dest, &[&remote]),
        Duration::from_secs(120),
    );
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 12);
    assert!(
        fake.counters.max_active.load(Ordering::SeqCst) >= 2,
        "more than one file at once"
    );
}

#[test]
fn transfer_engine_task_deep_trees_finish_at_limits_one_and_two() {
    let root = tempfile::tempdir().expect("temp dir");
    let local = deep_tree(&root.path().join("local"), 25);
    let remote = root.path().join("remote");
    fs::create_dir(&remote).expect("remote");
    let mut fake = Fake::new("deep-one");
    fake.ceiling = Some(1);
    let (_fake, handle) = fake.handle();
    let finished = run_within(
        job(
            Endpoint::Local,
            Endpoint::Remote(handle),
            &remote,
            &[&local],
        ),
        Duration::from_secs(120),
    );
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 25);

    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");
    let mut fake = Fake::new("deep-two");
    fake.ceiling = Some(2);
    let (_fake, handle) = fake.handle();
    let uploaded = remote.join("d0");
    let finished = run_within(
        job(
            Endpoint::Remote(handle),
            Endpoint::Local,
            &dest,
            &[&uploaded],
        ),
        Duration::from_secs(120),
    );
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 25);
    let mut deepest = dest.join("d0");
    for level in 1..25 {
        deepest = deepest.join(format!("d{level}"));
    }
    assert_eq!(
        fs::read(deepest.join("f24.txt")).expect("deepest"),
        b"level 24"
    );
}

#[test]
fn transfer_engine_task_two_jobs_on_one_connection_take_turns() {
    let root = tempfile::tempdir().expect("temp dir");
    for side in ["a", "b"] {
        for index in 0..6 {
            write(
                &root.path().join(format!("remote/{side}/f{index}.bin")),
                &vec![2u8; 300 * 1024],
            );
        }
        fs::create_dir(root.path().join(format!("dest-{side}"))).expect("dest");
    }
    let mut fake = Fake::new("fair");
    fake.ceiling = Some(1);
    fake.read_delay = Duration::from_millis(20);
    let (fake, handle) = fake.handle();
    let first = job(
        Endpoint::Remote(handle.clone()),
        Endpoint::Local,
        &root.path().join("dest-a"),
        &[&root.path().join("remote/a")],
    );
    let second = job(
        Endpoint::Remote(handle),
        Endpoint::Local,
        &root.path().join("dest-b"),
        &[&root.path().join("remote/b")],
    );
    let (done_tx, done_rx) = mpsc::channel();
    for transfer in [first, second] {
        let done_tx = done_tx.clone();
        std::thread::spawn(move || {
            let _ = done_tx.send(run(transfer));
        });
    }
    for _ in 0..2 {
        let finished = done_rx
            .recv_timeout(Duration::from_secs(120))
            .expect("both jobs finish");
        assert!(finished.issues.is_empty(), "{:?}", finished.issues);
        assert_eq!(finished.progress.files_done, 6);
    }
    let opened = fake.counters.opened.lock().expect("opened").clone();
    let last_a = opened
        .iter()
        .rposition(|path| path.contains("/remote/a/"))
        .expect("job a read");
    let first_b = opened
        .iter()
        .position(|path| path.contains("/remote/b/"))
        .expect("job b read");
    assert!(
        first_b < last_a,
        "the second job got turns early: {opened:?}"
    );
}

#[test]
fn transfer_engine_task_refused_read_access_ends_the_job() {
    let root = tempfile::tempdir().expect("temp dir");
    let target = fwd(root.path());
    let items = JobItems::Roots {
        paths: vec![fwd(&root.path().join("protected"))],
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
    let flow = crate::transfer::flow(unique("gate"), None);
    let folders = FolderRegister::new(Side::Local, &target, flow);
    let plan = super::roots::RootPlan::default();
    let cancel = AtomicBool::new(false);
    let report = |_: TransferMsg| {};
    let engine = Engine::new(view, TransferKind::Local, folders, &cancel, &report, &plan);
    let event = crate::transfer::walk::WalkEvent::AccessRefused {
        path: fwd(&root.path().join("protected/inner")),
    };
    assert!(
        !super::discovery::on_event(&engine, event),
        "the walk stops"
    );
    assert!(engine.stopped());
    let (issues, lines) = engine.issues.shown();
    assert_eq!(issues.len(), 1);
    assert!(issues[0].message.contains("Lesezugriff"));
    assert!(lines[0].contains("bereits Übertragenes bleibt erhalten"));
    if let Some(log) = engine.issues.log_path() {
        let _ = fs::remove_file(log);
    }
}

#[test]
fn transfer_engine_task_issue_log_lists_every_problem() {
    let root = tempfile::tempdir().expect("temp dir");
    let missing = [
        root.path().join("gone-1.txt"),
        root.path().join("gone-2.txt"),
    ];
    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");
    let finished = run(job(
        Endpoint::Local,
        Endpoint::Local,
        &dest,
        &[&missing[0], &missing[1]],
    ));
    assert_eq!(finished.progress.errors, 2);
    assert_eq!(finished.errors.len(), 2);
    let log = finished.progress.log_path.clone().expect("a log file");
    let text = fs::read_to_string(&log).expect("log readable");
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).expect("JSON line"))
        .collect();
    assert_eq!(lines.len(), 2);
    for line in &lines {
        let listed = line["path"].as_str().expect("path");
        assert!(
            missing.iter().any(|missing| fwd(missing) == listed),
            "{listed}"
        );
        assert!(!line["message"].as_str().expect("message").is_empty());
    }
    let _ = fs::remove_file(log);
}

#[test]
fn transfer_engine_task_lane_runs_engine_jobs() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("src/note.txt");
    write(&source, b"lane");
    let dest = root.path().join("dest");
    fs::create_dir(&dest).expect("dest");
    let request = TransferRequest::Job(Box::new(job(
        Endpoint::Local,
        Endpoint::Local,
        &dest,
        &[&source],
    )));
    let active = crate::transfer::launch_transfer(request).expect("launched");
    assert!(active.job.is_some(), "kept for transfer missing files");
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let done = loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match active
            .rx
            .recv_timeout(left)
            .expect("a terminal message in time")
        {
            TransferMsg::Progress(_) => {}
            TransferMsg::Done {
                progress, roots, ..
            } => break (progress, roots),
        }
    };
    assert_eq!(done.0.files_done, 1);
    assert_eq!(done.1.len(), 1);
    assert_eq!(fs::read(dest.join("note.txt")).expect("copied"), b"lane");
}
