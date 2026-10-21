//! Local copies on Windows (M-3 to M-5, M-6 "Windows"): the engine job against
//! `robocopy` (always available) and against the capacity of the drives
//! (`fio` with `--ioengine=windowsaio`; without `fio` only the requirement
//! against it is not run). Timing: the volume cache is flushed
//! (`Write-VolumeCache`) after every copy; the file cache cannot be emptied
//! before a run, which the report says. The last case confirms the PhysicalDisk
//! counter names U50 left open (REF § 9).
use crate::data::{random_file, small_tree, Scratch, GIB};
use crate::run::run_job;
use crate::support::{
    fast_dir, files_per_second, finish, guarded, job, retry_once, shortfall, slash, slow_dir,
    volume_capacity, Case, ReportLine, Trial, CASE_LIMIT,
};
use crate::tools::{run_tool, strings};
use smart_explorer::transfer::Endpoint;
use std::path::Path;
use std::time::{Duration, Instant};

const TOOL_LIMIT: Duration = Duration::from_secs(900);
const SMALL_FILES: usize = 50_000;
const SMALL_FOLDERS: usize = 500;

/// Flushes the volume cache of the drive holding `path` (best effort).
fn flush_volume(path: &Path) {
    let letter = path
        .to_string_lossy()
        .chars()
        .next()
        .filter(|letter| letter.is_ascii_alphabetic());
    if let Some(letter) = letter {
        let _ = run_tool(
            "powershell",
            &strings(&[
                "-NoProfile",
                "-Command",
                &format!("Write-VolumeCache -DriveLetter {letter}"),
            ]),
            &[],
            Duration::from_secs(120),
        );
    }
}

/// `robocopy` succeeded when its exit code is below 8; seconds include the flush.
fn robocopy(from: &Path, to: &Path, extra: &[&str], files: &[&str]) -> Result<f64, String> {
    let mut args = strings(&[&from.to_string_lossy(), &to.to_string_lossy()]);
    args.extend(strings(files));
    args.extend(strings(extra));
    args.extend(strings(&["/NFL", "/NDL", "/NJH", "/NJS", "/NP"]));
    let started = Instant::now();
    let done = run_tool("robocopy", &args, &[], TOOL_LIMIT)?;
    if done.timed_out || done.code.is_none_or(|code| code >= 8) {
        return Err(format!("robocopy: {}", done.describe()));
    }
    flush_volume(to);
    Ok(started.elapsed().as_secs_f64())
}

/// `speed_task_windows_local_*`: `small` selects 50 000 files instead of 4 GiB.
pub fn run_local(case: Case, same_volume: bool, small: bool) -> Result<(), String> {
    let fast = fast_dir()?;
    let target_root = if same_volume {
        fast.clone()
    } else {
        slow_dir()?
    };
    let fio = if small {
        files_per_second(&target_root)
    } else {
        volume_capacity(&fast).and_then(|source| {
            let target = volume_capacity(&target_root)?;
            let capacity = if same_volume {
                source.mix / 2.0
            } else {
                source.read.min(target.write)
            };
            Ok(capacity)
        })
    };
    let source_scratch = Scratch::new(&fast, case.name);
    let target_scratch = Scratch::new(&target_root, case.name);
    let (source, amount) = if small {
        let tree = small_tree(
            &source_scratch.sub("tree"),
            SMALL_FOLDERS,
            SMALL_FILES,
            4096,
        );
        (tree, SMALL_FILES as f64)
    } else {
        let file = source_scratch.path().join("large.bin");
        random_file(&file, 4 * GIB);
        (file, 4.0 * GIB as f64)
    };
    let reference_dir = target_scratch.sub("robocopy");
    let seconds = if small {
        robocopy(&source, &reference_dir, &["/E", "/MT:8"], &[])?
    } else {
        robocopy(
            source.parent().unwrap_or(&source),
            &reference_dir,
            &["/J"],
            &["large.bin"],
        )?
    };
    let reference = amount / seconds;
    let unit = if small { "files/s" } else { "B/s" };
    let trial = retry_once(|| {
        let target = target_scratch.sub(&format!("se-{}", crate::support::unique("t")));
        let started = Instant::now();
        let run = run_job(
            job(
                Endpoint::Local,
                Endpoint::Local,
                vec![slash(&source)],
                slash(&target),
                case.name,
            ),
            CASE_LIMIT,
        );
        flush_volume(&target);
        let elapsed = started.elapsed().as_secs_f64();
        let rate = amount / elapsed;
        let mut line = ReportLine::new(&case, unit);
        line.size_bytes = if small {
            SMALL_FILES as u64 * 4096
        } else {
            4 * GIB
        };
        line.files = run.files;
        line.se_rate = rate;
        line.reference_tool = "robocopy (/J für große Dateien, /MT:8 für Bäume)".into();
        line.reference_rate = reference;
        line.limiter = run.limiter.code().to_string();
        line.notes.push(format!(
            "{elapsed:.1} s mit Volumen-Flush; der Dateicache wurde nicht geleert"
        ));
        let mut failure = shortfall(
            case.name,
            rate,
            reference,
            0.95,
            "Rate von robocopy",
            "robocopy, gleiche Daten",
            unit,
        );
        match &fio {
            Ok(capacity) => {
                let (required, words, name) = if small {
                    (0.5, "≥ 0,5 × F_fio und ≥ 0,95 × robocopy", "F_fio")
                } else {
                    (0.85, "≥ 0,85 × C_copy und ≥ 0,95 × robocopy", "C_copy")
                };
                line.against(*capacity, required, words);
                failure = failure.or(shortfall(
                    case.name,
                    rate,
                    *capacity,
                    required,
                    name,
                    "fio --ioengine=windowsaio",
                    unit,
                ));
            }
            Err(reason) => {
                line.against(reference, 0.95, "≥ 0,95 × robocopy");
                line.skipped
                    .push(format!("Forderung gegen fio nicht ausgeführt: {reason}"));
            }
        }
        Trial { line, failure }
    });
    finish(&trial.line, trial.failure.as_deref());
    Ok(())
}

/// The PhysicalDisk counters of the runner (REF § 9): the case passes when a
/// counter named `Avg. Disk Queue Length` exists, and reports the names.
pub fn run_pdh() -> Result<(), String> {
    let name = "speed_task_windows_pdh_physicaldisk_counters_exist";
    let done = run_tool(
        "typeperf",
        &strings(&["-q", "PhysicalDisk"]),
        &[],
        Duration::from_secs(120),
    )?;
    let counters: Vec<&str> = done
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('\\'))
        .collect();
    let has_queue = counters
        .iter()
        .any(|counter| counter.contains("Avg. Disk Queue Length"));
    let case = Case::new(name, "windows", "-", "-");
    let mut line = ReportLine::new(&case, "B/s");
    line.requirement = "ein Zähler mit „Avg. Disk Queue Length“ im Namen".into();
    line.reference_tool = "typeperf -q PhysicalDisk".into();
    line.notes.push(format!(
        "{} Zähler; {}",
        counters.len(),
        counters
            .iter()
            .take(40)
            .copied()
            .collect::<Vec<_>>()
            .join(" | ")
    ));
    let failure = (!has_queue).then(|| {
        format!(
            "{name}: kein Zähler „Avg. Disk Queue Length“ unter {} Zählern von typeperf -q PhysicalDisk",
            counters.len()
        )
    });
    finish(&line, failure.as_deref());
    Ok(())
}

#[test]
#[ignore = "needs the stage's drives and robocopy"]
fn speed_task_windows_local_large_same_volume() {
    let case = Case::new(
        "speed_task_windows_local_large_same_volume",
        "local",
        "copy",
        "same",
    );
    guarded(case.name, || run_local(case, true, false));
}

#[test]
#[ignore = "needs the stage's drives and robocopy"]
fn speed_task_windows_local_large_two_volumes() {
    let case = Case::new(
        "speed_task_windows_local_large_two_volumes",
        "local",
        "copy",
        "two",
    );
    guarded(case.name, || run_local(case, false, false));
}

#[test]
#[ignore = "needs the stage's drives and robocopy"]
fn speed_task_windows_local_small_two_volumes() {
    let case = Case::new(
        "speed_task_windows_local_small_two_volumes",
        "local",
        "copy",
        "two",
    );
    guarded(case.name, || run_local(case, false, true));
}

#[test]
#[ignore = "needs typeperf of the Windows runner"]
fn speed_task_windows_pdh_physicaldisk_counters_exist() {
    guarded(
        "speed_task_windows_pdh_physicaldisk_counters_exist",
        run_pdh,
    );
}
