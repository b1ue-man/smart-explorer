//! Copy/paste of many small files on one device (K-A..K-E, 2026-10-28): the
//! GUI-equivalent job (crash log on) against the CLI-equivalent job (no
//! crash log) on the same 5000 files of 4 KiB in 50 folders, and the publish
//! queue of download packets (K-C). The benchmarks are ignored by default:
//! `cargo test --release -- --ignored speed_task_local --nocapture
//! --test-threads=1` prints one `BENCH <case> files_per_s=<n>` line each.
use super::download::Part;
use super::ops::Outcome;
use super::publish_queue::with_publisher;
use super::queue::FileWork;
use super::test_backend::{collect, fwd, job, run, write, Finished};
use super::{job_store, roots::RootPlan, side_flow, Engine, FolderRegister, JobView, Side};
use crate::transfer::{
    Endpoint, FolderConflict, FolderConflictPolicy, JobItems, Layout, RecoveryMode, TransferMsg,
};
use crate::types::{Conflict, CopyMode};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

const FILES: usize = 5000;
const FOLDERS: usize = 50;
const FILE_BYTES: usize = 4096;

/// 5000 files of 4 KiB in 50 folders below `root/source/tree`.
fn tree(root: &Path) -> PathBuf {
    let tree = root.join("source").join("tree");
    let bytes = vec![0x5Au8; FILE_BYTES];
    for index in 0..FILES {
        let folder = tree.join(format!("d{:02}", index % FOLDERS));
        write(&folder.join(format!("f{index:05}.bin")), &bytes);
    }
    tree
}

/// The job `se cp` runs (cli/engine_copy.rs): no crash log, skip, merge.
fn cli_equivalent(tree: &Path, target: &Path) -> Finished {
    let items = JobItems::Roots {
        paths: vec![fwd(tree)],
        base: None,
    };
    let folders = FolderConflictPolicy::new(FolderConflict::Merge);
    let target_dir = fwd(target);
    let view = JobView {
        recoverable: false,
        recovery: RecoveryMode::New,
        id: None,
        folders: &folders,
        source: Side::Local,
        target: Side::Local,
        target_dir: &target_dir,
        items: &items,
        layout: Layout::Tree,
        filter: None,
        conflict: Conflict::Skip,
        mode: CopyMode::Copy,
        source_label: "Quelle",
        target_label: "Ziel",
        resume: None,
    };
    let (tx, rx) = crossbeam_channel::unbounded();
    let cancel = AtomicBool::new(false);
    let report = move |message: TransferMsg| {
        let _ = tx.send(message);
    };
    super::run_view(view, &report, &cancel);
    collect(&rx)
}

/// Copies the tree once and prints its rate.
fn bench(case: &str, recoverable: bool) {
    let root = tempfile::tempdir().expect("temp dir");
    let tree = tree(root.path());
    let target = root.path().join("target");
    std::fs::create_dir(&target).expect("target");
    let started = Instant::now();
    let finished = if recoverable {
        // `job` is the GUI's paste job: crash log, keep both, rename.
        run(job(Endpoint::Local, Endpoint::Local, &target, &[&tree]))
    } else {
        cli_equivalent(&tree, &target)
    };
    let seconds = started.elapsed().as_secs_f64().max(1e-9);
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, FILES as u64);
    println!("BENCH {case} files_per_s={:.0}", FILES as f64 / seconds);
}

#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture"]
fn speed_task_local_gui_recoverable_5000_small_files() {
    bench("local_gui_recoverable", true);
}

#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture"]
fn speed_task_local_cli_unrecoverable_5000_small_files() {
    bench("local_cli_unrecoverable", false);
}

#[test]
fn speed_task_publish_queue_publishes_off_the_stream_without_replacing() {
    let root = tempfile::tempdir().expect("temp dir");
    let source = root.path().join("a.txt");
    let target = root.path().join("target");
    write(&source, b"payload");
    write(&target.join("a.txt"), b"foreign");
    let candidate = job(Endpoint::Local, Endpoint::Local, &target, &[&source]);
    let cancel = AtomicBool::new(false);
    let report = |_| {};
    let view = JobView::of(&candidate);
    let folders = FolderRegister::new(
        view.target,
        view.target_dir,
        side_flow(view.target, view.target_dir),
    );
    let mut engine = Engine::new(
        view,
        view.kind(),
        folders,
        &cancel,
        &report,
        &RootPlan::default(),
    );
    engine.job_record = job_store::begin(&view, view.target_dir, &cancel).expect("job");
    let meta = std::fs::metadata(&source).expect("source");
    let file = FileWork::new(
        fwd(&source),
        "a.txt".into(),
        meta.len(),
        crate::local_access::system_time_ms(meta.modified().expect("mtime")),
    );
    let destination = target.join("a.txt");
    let ((), published) = with_publisher(&engine, |publisher| {
        for index in 0..2 {
            let mut part = Part::create(&destination).expect("part");
            part.write(b"payload").expect("bytes");
            publisher.submit(index, part, destination.clone(), file.clone());
        }
    });
    assert_eq!(published.len(), 2);
    for (_, result) in published {
        assert_eq!(result.expect("published"), Outcome::Done);
    }
    assert_eq!(engine.issues.total(), 0);
    // Keep both: the foreign file stays, both parts got their own number.
    assert_eq!(std::fs::read(&destination).expect("foreign"), b"foreign");
    for name in ["a (2).txt", "a (3).txt"] {
        assert_eq!(std::fs::read(target.join(name)).expect(name), b"payload");
    }
}
