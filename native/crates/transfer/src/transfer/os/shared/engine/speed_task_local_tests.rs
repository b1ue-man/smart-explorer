//! Throughput of copy and move on one device (KP4, 2026-10-28), from the
//! first byte to the last, for three data sets: 5000 files of 4 KiB in 50
//! folders, 1000 files of 1 MiB in 10 folders and one file of 2 GiB. Each
//! data set runs as the GUI's paste job (crash log, keep both), as the job
//! `se cp` runs (no crash log), as a GUI move on the same volume (into an
//! existing folder, so every file moves alone) and as a GUI move to another
//! volume when the runner has one (`SE_BENCH_SECOND_VOLUME`, or `/dev/shm`
//! on Linux when it is another file system). A reference line times the
//! platform's own tool on the same data (`cp -r` on Linux, `robocopy /E
//! /MT:16` on Windows) where it exists.
//!
//! All are ignored by default: `cargo test --release -- --ignored
//! speed_task_local_ --nocapture --test-threads=1` prints one
//! `BENCH <case> files_per_s=<n> mib_per_s=<n>` line per case (a case that
//! cannot run prints `BENCH <case> skipped=<reason>`).
use super::test_backend::{job, run, write};
use crate::transfer::{Endpoint, FolderConflict, FolderConflictPolicy};
use crate::types::CopyMode;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// One data set.
#[derive(Clone, Copy)]
struct Data {
    name: &'static str,
    files: usize,
    folders: usize,
    bytes: u64,
}

const SMALL: Data = Data {
    name: "5000x4k",
    files: 5000,
    folders: 50,
    bytes: 4096,
};
const MEDIUM: Data = Data {
    name: "1000x1m",
    files: 1000,
    folders: 10,
    bytes: 1024 * 1024,
};
const LARGE: Data = Data {
    name: "1x2g",
    files: 1,
    folders: 1,
    bytes: 2 * 1024 * 1024 * 1024,
};

/// How a case runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Way {
    Gui,
    Cli,
    MoveSame,
    MoveCross,
}

impl Way {
    fn name(self) -> &'static str {
        match self {
            Way::Gui => "gui",
            Way::Cli => "cli",
            Way::MoveSame => "move_same_volume",
            Way::MoveCross => "move_cross_volume",
        }
    }
}

/// The data set below `root/tree`; returns the tree.
fn fill(root: &Path, data: Data) -> PathBuf {
    let tree = root.join("tree");
    // One block written repeatedly: 8 MiB at most in memory.
    let block = vec![0x5Au8; data.bytes.min(8 * 1024 * 1024) as usize];
    for index in 0..data.files {
        let folder = tree.join(format!("d{:02}", index % data.folders.max(1)));
        let path = folder.join(format!("f{index:05}.bin"));
        if data.bytes as usize == block.len() {
            write(&path, &block);
            continue;
        }
        std::fs::create_dir_all(&folder).expect("fixture folder");
        let mut file = std::fs::File::create(&path).expect("fixture file");
        let mut left = data.bytes;
        while left > 0 {
            let chunk = left.min(block.len() as u64) as usize;
            std::io::Write::write_all(&mut file, &block[..chunk]).expect("fixture bytes");
            left -= chunk as u64;
        }
    }
    tree
}

/// A folder on another file system than `near`, when the runner has one.
fn second_volume(near: &Path) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SE_BENCH_SECOND_VOLUME") {
        return Some(PathBuf::from(path));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let shm = Path::new("/dev/shm");
        let here = std::fs::metadata(near).ok()?.dev();
        let there = std::fs::metadata(shm).ok()?.dev();
        (here != there).then(|| shm.to_path_buf())
    }
    #[cfg(not(unix))]
    {
        let _ = near;
        None
    }
}

fn print(case: &str, data: Data, seconds: f64) {
    let seconds = seconds.max(1e-9);
    let total = data.bytes as f64 * data.files as f64;
    println!(
        "BENCH {case} files_per_s={:.0} mib_per_s={:.1}",
        data.files as f64 / seconds,
        total / seconds / (1024.0 * 1024.0)
    );
}

/// The job `se cp` runs (no crash log, skip, merge): the GUI job's shape
/// with `recoverable` off.
fn cli_job(tree: &Path, target: &Path) -> crate::transfer::TransferJob {
    let mut cli = job(Endpoint::Local, Endpoint::Local, target, &[tree]);
    cli.recoverable = false;
    cli.folders = FolderConflictPolicy::new(FolderConflict::Merge);
    cli.conflict = crate::types::Conflict::Skip;
    cli
}

/// Runs one case and prints its line.
fn bench(data: Data, way: Way) {
    let case = format!("local_{}_{}", data.name, way.name());
    let root = tempfile::tempdir().expect("temp dir");
    let tree = fill(root.path(), data);
    let other = match way {
        Way::MoveCross => match second_volume(root.path()) {
            Some(volume) => Some(tempfile::tempdir_in(volume).expect("second volume dir")),
            None => {
                println!("BENCH {case} skipped=no_second_volume");
                return;
            }
        },
        _ => None,
    };
    let target = match &other {
        Some(other) => other.path().join("target"),
        None => root.path().join("target"),
    };
    std::fs::create_dir(&target).expect("target");
    let candidate = match way {
        Way::Gui => job(Endpoint::Local, Endpoint::Local, &target, &[&tree]),
        Way::Cli => cli_job(&tree, &target),
        Way::MoveSame | Way::MoveCross => {
            let mut moving = job(Endpoint::Local, Endpoint::Local, &target, &[&tree]);
            moving.mode = CopyMode::Move;
            // An existing folder of the same name: no whole-folder rename,
            // every file moves alone (the per-file path is what is timed).
            moving.folders = FolderConflictPolicy::new(FolderConflict::Merge);
            std::fs::create_dir(target.join("tree")).expect("existing folder");
            moving
        }
    };
    let started = Instant::now();
    let finished = run(candidate);
    let seconds = started.elapsed().as_secs_f64();
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, data.files as u64);
    print(&case, data, seconds);
    if way == Way::Cli {
        reference(data, &tree, root.path());
    }
}

/// The platform's own tool on the same tree, into a fresh folder.
fn reference(data: Data, tree: &Path, root: &Path) {
    let target = root.join("reference");
    let started = Instant::now();
    #[cfg(windows)]
    let (tool, status) = (
        "robocopy",
        std::process::Command::new("robocopy")
            .arg(tree)
            .arg(&target)
            .args(["/E", "/MT:16", "/NFL", "/NDL", "/NJH", "/NJS", "/NP"])
            .status()
            // robocopy: below 8 is success (1 = files copied).
            .map(|status| status.code().is_some_and(|code| code < 8)),
    );
    #[cfg(not(windows))]
    let (tool, status) = (
        "cp",
        std::process::Command::new("cp")
            .arg("-r")
            .arg(tree)
            .arg(&target)
            .status()
            .map(|status| status.success()),
    );
    let seconds = started.elapsed().as_secs_f64();
    let case = format!("local_{}_reference_{tool}", data.name);
    match status {
        Ok(true) => print(&case, data, seconds),
        Ok(false) => println!("BENCH {case} skipped=tool_failed"),
        Err(_) => println!("BENCH {case} skipped=tool_missing"),
    }
}

macro_rules! benches {
    ($($name:ident => $data:expr, $way:expr;)*) => {
        $(
            #[test]
            #[ignore = "benchmark: run with --release --ignored --nocapture"]
            fn $name() {
                bench($data, $way);
            }
        )*
    };
}

benches! {
    speed_task_local_small_gui => SMALL, Way::Gui;
    speed_task_local_small_cli => SMALL, Way::Cli;
    speed_task_local_small_move_same_volume => SMALL, Way::MoveSame;
    speed_task_local_small_move_cross_volume => SMALL, Way::MoveCross;
    speed_task_local_medium_gui => MEDIUM, Way::Gui;
    speed_task_local_medium_cli => MEDIUM, Way::Cli;
    speed_task_local_medium_move_same_volume => MEDIUM, Way::MoveSame;
    speed_task_local_medium_move_cross_volume => MEDIUM, Way::MoveCross;
    speed_task_local_large_gui => LARGE, Way::Gui;
    speed_task_local_large_cli => LARGE, Way::Cli;
    speed_task_local_large_move_same_volume => LARGE, Way::MoveSame;
    speed_task_local_large_move_cross_volume => LARGE, Way::MoveCross;
}

/// The data sets and ways are what the benchmarks promise (runs always).
#[test]
fn speed_task_local_matrix_names_every_case() {
    assert_eq!(SMALL.files as u64 * SMALL.bytes, 5000 * 4096);
    assert_eq!(MEDIUM.files as u64 * MEDIUM.bytes, 1000 * 1024 * 1024);
    assert_eq!(LARGE.files as u64 * LARGE.bytes, 2 * 1024 * 1024 * 1024);
    let names: Vec<&str> = [Way::Gui, Way::Cli, Way::MoveSame, Way::MoveCross]
        .iter()
        .map(|way| way.name())
        .collect();
    assert_eq!(
        names,
        ["gui", "cli", "move_same_volume", "move_cross_volume"]
    );
}
