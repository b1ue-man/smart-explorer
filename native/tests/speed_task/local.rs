//! Local copies on Linux (M-3 to M-5, M-6 "Lokal Linux"): the engine job
//! against the capacity of the drives (`fio`) and against `cp` and `rsync`.
//! Timing as in M-4: `sync; drop_caches` before, `syncfs` on the target after
//! (Smart Explorer itself does not sync a new copy). The clone case runs on
//! the stage's XFS volume (`mkfs.xfs -m reflink=1` on a loop device).
use crate::data::{random_file, small_tree, Scratch, GIB};
use crate::reference::fio_files;
use crate::run::{run_job, Run};
use crate::support::{
    fast_dir, finish, guarded, job, need, retry_once, shortfall, slash, slow_dir, volume_capacity,
    Case, ReportLine, Trial, CASE_LIMIT,
};
use crate::tools::{drop_caches, run_tool, strings, syncfs};
use smart_explorer::transfer::Endpoint;
use std::path::Path;
use std::time::{Duration, Instant};

const TOOL_LIMIT: Duration = Duration::from_secs(900);
const SMALL_FILES: usize = 50_000;
const SMALL_FOLDERS: usize = 500;

/// The engine copies `source` into `target_dir`; seconds include `syncfs`.
fn se_copy(case: &Case, source: &Path, target_dir: &Path) -> (Run, f64) {
    drop_caches().expect("the page cache must be empty before a local run");
    let started = Instant::now();
    let run = run_job(
        job(
            Endpoint::Local,
            Endpoint::Local,
            vec![slash(source)],
            slash(target_dir),
            case.name,
        ),
        CASE_LIMIT,
    );
    syncfs(target_dir).expect("sync the target file system");
    (run, started.elapsed().as_secs_f64())
}

/// A comparison tool copies; seconds include the cache drop's aftermath
/// (`syncfs`), not the drop itself.
fn tool_copy(program: &str, args: &[String], target_dir: &Path) -> Result<f64, String> {
    drop_caches()?;
    let started = Instant::now();
    let done = run_tool(program, args, &[], TOOL_LIMIT)?;
    if !done.ok() {
        return Err(format!("{program}: {}", done.describe()));
    }
    syncfs(target_dir)?;
    Ok(started.elapsed().as_secs_f64())
}

fn version_of(program: &str) -> String {
    run_tool(
        program,
        &strings(&["--version"]),
        &[],
        Duration::from_secs(30),
    )
    .map(|done| done.stdout.lines().next().unwrap_or("").to_string())
    .unwrap_or_default()
}

/// Whether the volume of `dir` clones files (`cp --reflink=always`).
fn clones(dir: &Path) -> bool {
    let probe = dir.join("clone-probe");
    let _ = std::fs::write(&probe, b"probe");
    let copy = dir.join("clone-probe-copy");
    let done = run_tool(
        "cp",
        &strings(&[
            "--reflink=always",
            &probe.to_string_lossy(),
            &copy.to_string_lossy(),
        ]),
        &[],
        Duration::from_secs(30),
    );
    let _ = std::fs::remove_file(&probe);
    let _ = std::fs::remove_file(&copy);
    matches!(done, Ok(done) if done.ok())
}

/// `speed_task_local_large_{same_volume,two_volumes}`: 4 GiB.
pub fn run_large(case: Case, same_volume: bool) -> Result<(), String> {
    let fast = fast_dir()?;
    let target_root = if same_volume {
        fast.clone()
    } else {
        slow_dir()?
    };
    let source_volume = volume_capacity(&fast)?;
    let target_volume = volume_capacity(&target_root)?;
    let (copy_capacity, name, tool) = if same_volume {
        (source_volume.mix / 2.0, "C_mix ÷ 2", "fio --rw=rw")
    } else {
        (
            source_volume.read.min(target_volume.write),
            "min(C_read, C_write)",
            "fio --rw=read / --rw=write",
        )
    };
    drop_caches()?;
    let source_scratch = Scratch::new(&fast, case.name);
    let source = source_scratch.path().join("large.bin");
    random_file(&source, 4 * GIB);
    let target_scratch = Scratch::new(&target_root, case.name);
    // On a file system with cloning the capacity row does not apply (M-5).
    let cloning = clones(&target_root);
    let copy_dir = target_scratch.sub("cp");
    let reference = 4.0 * GIB as f64
        / tool_copy(
            "cp",
            &strings(&[
                "--reflink=never",
                &source.to_string_lossy(),
                &copy_dir.to_string_lossy(),
            ]),
            &copy_dir,
        )?;
    let cp_version = version_of("cp");
    let trial = retry_once(|| {
        let target = target_scratch.sub(&format!("se-{}", crate::support::unique("t")));
        let (run, seconds) = se_copy(&case, &source, &target);
        assert_eq!(run.bytes, 4 * GIB, "{}: every byte arrives", case.name);
        let rate = run.bytes as f64 / seconds;
        let mut line = ReportLine::new(&case, "B/s");
        line.size_bytes = run.bytes;
        line.files = run.files;
        line.se_rate = rate;
        line.against(
            copy_capacity,
            0.85,
            "≥ 0,85 × C_copy und ≥ 0,95 × Rate von cp",
        );
        line.reference_tool = format!("{tool}; cp --reflink=never ({cp_version})");
        line.reference_rate = reference;
        line.limiter = run.limiter.code().to_string();
        line.notes.push(format!(
            "{seconds:.1} s mit syncfs, Bereiche höchstens {}, {name}",
            run.max_segments
        ));
        let mut failure = shortfall(
            case.name,
            rate,
            reference,
            0.95,
            "Rate von cp",
            "cp --reflink=never, gleiche Datei",
            "B/s",
        );
        if cloning {
            line.notes
                .push("das Laufwerk klont: die Forderung an C_copy entfällt (M-5)".into());
        } else {
            failure = failure.or(shortfall(
                case.name,
                rate,
                copy_capacity,
                0.85,
                name,
                tool,
                "B/s",
            ));
        }
        Trial { line, failure }
    });
    finish(&trial.line, trial.failure.as_deref());
    Ok(())
}

/// `speed_task_local_small_{same_volume,two_volumes}`: 50 000 files.
pub fn run_small(case: Case, same_volume: bool) -> Result<(), String> {
    let fast = fast_dir()?;
    let target_root = if same_volume {
        fast.clone()
    } else {
        slow_dir()?
    };
    let fio_rate = fio_files(&target_root)?;
    drop_caches()?;
    let source_scratch = Scratch::new(&fast, case.name);
    let tree = small_tree(
        &source_scratch.sub("tree"),
        SMALL_FOLDERS,
        SMALL_FILES,
        4096,
    );
    let target_scratch = Scratch::new(&target_root, case.name);
    let files = SMALL_FILES as f64;
    let cp_dir = target_scratch.sub("cp");
    let cp_rate = files
        / tool_copy(
            "cp",
            &strings(&[
                "-r",
                "--reflink=never",
                &tree.to_string_lossy(),
                &cp_dir.to_string_lossy(),
            ]),
            &cp_dir,
        )?;
    let rsync_dir = target_scratch.sub("rsync");
    let rsync_rate = run_tool_rsync(&tree, &rsync_dir).map(|seconds| files / seconds);
    let (reference, tool) = match &rsync_rate {
        Ok(rate) if *rate > cp_rate => (*rate, "rsync -a"),
        _ => (cp_rate, "cp -r"),
    };
    let trial = retry_once(|| {
        let target = target_scratch.sub(&format!("se-{}", crate::support::unique("t")));
        let (run, seconds) = se_copy(&case, &tree, &target);
        assert_eq!(run.files, SMALL_FILES as u64, "{}: every file", case.name);
        let rate = files / seconds;
        let mut line = ReportLine::new(&case, "files/s");
        line.size_bytes = SMALL_FILES as u64 * 4096;
        line.files = run.files;
        line.se_rate = rate;
        line.against(
            fio_rate,
            0.5,
            "≥ 1,0 × max(cp -r, rsync -a) und ≥ 0,5 × F_fio",
        );
        line.reference_tool = format!("{tool} (cp {cp_rate:.0}, rsync {:?})", rsync_rate);
        line.reference_rate = reference;
        line.limiter = run.limiter.code().to_string();
        line.notes.push(format!("{seconds:.1} s mit syncfs"));
        if let Err(reason) = &rsync_rate {
            line.skipped
                .push(format!("Vergleich mit rsync nicht ausgeführt: {reason}"));
        }
        let failure = shortfall(
            case.name,
            rate,
            reference,
            1.0,
            tool,
            "gleiche Dateien",
            "files/s",
        )
        .or(shortfall(
            case.name,
            rate,
            fio_rate,
            0.5,
            "F_fio",
            "fio --nrfiles=5000 --filesize=4k",
            "files/s",
        ));
        Trial { line, failure }
    });
    finish(&trial.line, trial.failure.as_deref());
    Ok(())
}

fn run_tool_rsync(tree: &Path, target: &Path) -> Result<f64, String> {
    tool_copy(
        "rsync",
        &strings(&[
            "-a",
            &format!("{}/", tree.to_string_lossy()),
            &format!("{}/", target.to_string_lossy()),
        ]),
        target,
    )
}

/// `speed_task_local_reflink_xfs`: cloning is a metadata operation; the copy
/// takes at most 5 % of the time of `cp --reflink=never` (M-5).
pub fn run_reflink(case: Case) -> Result<(), String> {
    let xfs = std::path::PathBuf::from(need(
        "SE_SPEED_XFS_DIR",
        "Einhängepunkt des XFS-Loop-Geräts",
    )?);
    drop_caches()?;
    let scratch = Scratch::new(&xfs, case.name);
    let source = scratch.path().join("large.bin");
    random_file(&source, 4 * GIB);
    assert!(
        clones(&xfs),
        "{}: das XFS der Stufe klont nicht (cp --reflink=always scheitert)",
        case.name
    );
    let plain_dir = scratch.sub("cp");
    let plain = tool_copy(
        "cp",
        &strings(&[
            "--reflink=never",
            &source.to_string_lossy(),
            &plain_dir.to_string_lossy(),
        ]),
        &plain_dir,
    )?;
    let reference = 4.0 * GIB as f64 / plain;
    let trial = retry_once(|| {
        let target = scratch.sub(&format!("se-{}", crate::support::unique("t")));
        let (run, seconds) = se_copy(&case, &source, &target);
        let rate = run.bytes as f64 / seconds;
        let mut line = ReportLine::new(&case, "B/s");
        line.size_bytes = run.bytes;
        line.files = run.files;
        line.se_rate = rate;
        line.against(reference, 20.0, "Kopierzeit ≤ 5 % von cp --reflink=never");
        line.reference_tool = "cp --reflink=never, gleiche Datei".into();
        line.reference_rate = reference;
        line.limiter = run.limiter.code().to_string();
        line.notes
            .push(format!("se {seconds:.2} s, cp {plain:.2} s"));
        let failure = shortfall(
            case.name,
            rate,
            reference,
            20.0,
            "Rate von cp --reflink=never",
            "gleiche Datei",
            "B/s",
        );
        Trial { line, failure }
    });
    finish(&trial.line, trial.failure.as_deref());
    Ok(())
}

#[test]
#[ignore = "needs the stage's drives, fio capacities and sudo"]
fn speed_task_local_large_same_volume() {
    let case = Case::new(
        "speed_task_local_large_same_volume",
        "local",
        "copy",
        "same",
    );
    guarded(case.name, || run_large(case, true));
}

#[test]
#[ignore = "needs the stage's drives, fio capacities and sudo"]
fn speed_task_local_large_two_volumes() {
    let case = Case::new("speed_task_local_large_two_volumes", "local", "copy", "two");
    guarded(case.name, || run_large(case, false));
}

#[test]
#[ignore = "needs the stage's drives, fio capacities and sudo"]
fn speed_task_local_small_same_volume() {
    let case = Case::new(
        "speed_task_local_small_same_volume",
        "local",
        "copy",
        "same",
    );
    guarded(case.name, || run_small(case, true));
}

#[test]
#[ignore = "needs the stage's drives, fio capacities and sudo"]
fn speed_task_local_small_two_volumes() {
    let case = Case::new("speed_task_local_small_two_volumes", "local", "copy", "two");
    guarded(case.name, || run_small(case, false));
}

#[test]
#[ignore = "needs the stage's XFS volume and sudo"]
fn speed_task_local_reflink_xfs() {
    let case = Case::new("speed_task_local_reflink_xfs", "local", "clone", "xfs");
    guarded(case.name, || run_reflink(case));
}
