//! The start of a job (M-5 row "Anlauf", M-6 "Anlauf"): 10 000 small files
//! over the `wan` link. The rate in the second second must reach 0.8 of the
//! steady rate and the first file must arrive within 1 s + 10 round trips
//! (TE1); with the hint file of an earlier job (a new process) the first
//! second must already reach 0.5 of the steady rate.
use crate::data::{small_tree, Scratch};
use crate::remote::{connect, Remote, Way};
use crate::run::{run_job, Run};
use crate::support::{
    check_profile, fast_dir, finish, guarded, job, need, retry_once, rtt_seconds, shortfall, slash,
    var, Case, ReportLine, Trial, CASE_LIMIT,
};
use crate::tools::{exec, strings};
use serde_json::{json, Value};
use smart_explorer::transfer::Endpoint;
use std::path::Path;
use std::time::Duration;

const FILES: usize = 10_000;
const FOLDERS: usize = 100;
const FILE_BYTES: usize = 4096;

/// The rates a ramp case compares, in bytes per second.
struct Series {
    first_second: f64,
    second_second: f64,
    steady: f64,
    first_file: Duration,
    note: String,
}

/// Steady rate = from three seconds on (or the second half of a short job);
/// a job shorter than 2.5 s shows no ramp, its second second is the steady rate.
fn series(run: &Run) -> Series {
    let end = run.total.as_millis() as u64;
    let from = 3000u64.min(end / 2);
    let steady = run.rate_between(from, end);
    let short = end < 2500;
    Series {
        first_second: run.rate_between(0, 1000),
        second_second: if short {
            steady
        } else {
            run.rate_between(1000, 2000)
        },
        steady,
        first_file: run.first_file,
        note: format!(
            "Lauf {:.1} s, Dauerrate ab {:.1} s{}",
            run.seconds(),
            from as f64 / 1000.0,
            if short {
                " (Lauf zu kurz für die zweite Sekunde)"
            } else {
                ""
            }
        ),
    }
}

fn upload(remote: &Remote, tree: &Path, label: &str) -> Run {
    let target = remote.guarded_folder("ramp");
    let run = run_job(
        job(
            Endpoint::Local,
            Endpoint::Remote(remote.backend.clone()),
            vec![slash(tree)],
            target.path.clone(),
            label,
        ),
        CASE_LIMIT,
    );
    assert_eq!(run.files, FILES as u64, "{label}: every file arrives");
    run
}

/// `speed_task_ramp_{sftp,ftp}_small_wan`.
pub fn run_ramp(case: Case) -> Result<(), String> {
    check_profile(&case)?;
    let rtt = rtt_seconds().ok_or("SE_SPEED_RTT_MS fehlt (Rundreisezeit der Strecke)")?;
    let remote = connect(Way::parse(case.way))?;
    let scratch = Scratch::new(&fast_dir()?, case.name);
    let tree = small_tree(&scratch.sub("tree"), FOLDERS, FILES, FILE_BYTES);
    let bound = Duration::from_secs(1) + Duration::from_secs_f64(rtt * 10.0);
    let trial = retry_once(|| {
        let run = upload(&remote, &tree, case.name);
        let found = series(&run);
        let mut line = ReportLine::new(&case, "B/s");
        line.size_bytes = (FILES * FILE_BYTES) as u64;
        line.files = run.files;
        line.se_rate = found.second_second;
        line.against(found.steady, 0.8, "zweite Sekunde ≥ 0,8 × Dauerrate");
        line.reference_tool = "Dauerrate des Laufs".into();
        line.reference_rate = found.steady;
        line.limiter = run.limiter.code().to_string();
        line.notes.push(format!(
            "erste Datei nach {:.2} s (Grenze {:.2} s); {}",
            found.first_file.as_secs_f64(),
            bound.as_secs_f64(),
            found.note
        ));
        let mut failure = shortfall(
            case.name,
            found.second_second,
            found.steady,
            0.8,
            "Dauerrate",
            "gleicher Lauf",
            "B/s",
        );
        if found.first_file > bound {
            failure = failure.or(Some(format!(
                "{}: erste Datei nach {:?} > {:?} (1 s + 10 RTT)",
                case.name, found.first_file, bound
            )));
        }
        Trial { line, failure }
    });
    finish(&trial.line, trial.failure.as_deref());
    Ok(())
}

/// The helper of `speed_task_ramp_persisted_hint_sftp_wan`: one more job in a
/// new process that finds the hint file of the first one.
#[test]
#[ignore = "helper of speed_task_ramp_persisted_hint_sftp_wan, started by that case"]
fn speed_task_ramp_child() {
    let Some(out) = var("SE_SPEED_CHILD_OUT") else {
        println!("kein SE_SPEED_CHILD_OUT: Hilfsfall ohne Aufrufer, nichts zu tun");
        return;
    };
    let remote = connect(Way::Sftp).expect("the helper connects to the SFTP server");
    let scratch = Scratch::new(&fast_dir().expect("fast folder"), "ramp-child");
    let tree = small_tree(&scratch.sub("tree"), FOLDERS, FILES, FILE_BYTES);
    let run = upload(&remote, &tree, "speed_task_ramp_child");
    let found = series(&run);
    let result = json!({
        "first_second": found.first_second, "steady": found.steady,
        "first_file_s": found.first_file.as_secs_f64(), "note": found.note,
    });
    std::fs::write(out, result.to_string()).expect("write the helper result");
}

/// A first job learns the limit and writes the hint file; the same job in a
/// new process starts from it.
pub fn run_persisted_hint(case: Case) -> Result<(), String> {
    check_profile(&case)?;
    need("SE_SPEED_SFTP", "SFTP-Server der Stufe")?;
    let remote = connect(Way::Sftp)?;
    let scratch = Scratch::new(&fast_dir()?, case.name);
    let tree = small_tree(&scratch.sub("tree"), FOLDERS, FILES, FILE_BYTES);
    let learned = series(&upload(&remote, &tree, case.name));
    // The hint file is written when the first job ends.
    std::thread::sleep(Duration::from_secs(1));
    let out = scratch.path().join("child.json");
    let program = std::env::current_exe().map_err(|error| error.to_string())?;
    let args = strings(&[
        "--ignored",
        "--exact",
        "ramp::speed_task_ramp_child",
        "--test-threads=1",
        "--nocapture",
    ]);
    let envs = vec![(
        "SE_SPEED_CHILD_OUT".to_string(),
        out.to_string_lossy().to_string(),
    )];
    let done = exec(&program.to_string_lossy(), &args, &envs, CASE_LIMIT)
        .map_err(|error| format!("Hilfsprozess nicht startbar: {error}"))?;
    assert!(
        done.ok(),
        "{}: Hilfsprozess: {}",
        case.name,
        done.describe()
    );
    let text = std::fs::read_to_string(&out).unwrap_or_default();
    let found: Value = serde_json::from_str(&text)
        .map_err(|error| format!("{}: Ergebnis des Hilfsprozesses fehlt: {error}", case.name))
        .expect("the helper wrote its result");
    let first = found["first_second"].as_f64().unwrap_or(0.0);
    let steady = found["steady"].as_f64().unwrap_or(0.0);
    let mut line = ReportLine::new(&case, "B/s");
    line.size_bytes = (FILES * FILE_BYTES) as u64;
    line.files = FILES as u64;
    line.se_rate = first;
    line.against(
        steady,
        0.5,
        "erste Sekunde ≥ 0,5 × Dauerrate (mit Hinweisdatei)",
    );
    line.reference_tool = "Dauerrate des Hilfsprozesses".into();
    line.reference_rate = steady;
    line.notes.push(format!(
        "erster Lauf: zweite Sekunde {:.1} MB/s, Dauerrate {:.1} MB/s; Hilfsprozess: {}",
        learned.second_second / 1e6,
        learned.steady / 1e6,
        found["note"].as_str().unwrap_or("")
    ));
    let failure = shortfall(
        case.name,
        first,
        steady,
        0.5,
        "Dauerrate",
        "Lauf im neuen Prozess",
        "B/s",
    );
    finish(&line, failure.as_deref());
    Ok(())
}

#[test]
#[ignore = "needs the stage's SFTP server and the shaped wan link"]
fn speed_task_ramp_sftp_small_wan() {
    let case = Case::new("speed_task_ramp_sftp_small_wan", "sftp", "up", "wan");
    guarded(case.name, || run_ramp(case));
}

#[test]
#[ignore = "needs the stage's FTP server and the shaped wan link"]
fn speed_task_ramp_ftp_small_wan() {
    let case = Case::new("speed_task_ramp_ftp_small_wan", "ftp", "up", "wan");
    guarded(case.name, || run_ramp(case));
}

#[test]
#[ignore = "needs the stage's SFTP server and the shaped wan link"]
fn speed_task_ramp_persisted_hint_sftp_wan() {
    let case = Case::new(
        "speed_task_ramp_persisted_hint_sftp_wan",
        "sftp",
        "up",
        "wan",
    );
    guarded(case.name, || run_persisted_hint(case));
}
