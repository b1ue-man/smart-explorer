//! The unshaped line (M-5 rows "raw" and "Chiffre", M-6 "Ungeformt" and
//! "Chiffre"): there the cost of Smart Explorer itself shows. One large file
//! up and down over SFTP, FTP and WebDAV against the plain client of the same
//! protocol on the same file and server (`sftp -b`, `curl`), and the SFTP
//! default against a server that allows only one cipher.
use crate::data::{random_file, Scratch, GIB};
use crate::remote::{connect, connect_sftp_vars, Remote, Way};
use crate::run::{run_job, Run};
use crate::support::{
    check_profile, fast_dir, finish, guarded, job, need, retry_once, shortfall, slash, Case,
    ReportLine, Trial, CASE_LIMIT,
};
use crate::tools::{run_tool, strings};
use smart_explorer::transfer::Endpoint;
use std::path::Path;
use std::time::Duration;

/// The size of a raw case: the unshaped loopback is fast, three seconds of
/// transfer still dwarf the start of the job.
const RAW_BYTES: u64 = 3 * GIB;
const CLIENT_LIMIT: Duration = Duration::from_secs(600);

/// One engine job moving the large file up to `remote` or down from `file`.
fn se_run(case: &Case, remote: &Remote, source: &Path, scratch: &Scratch, file: &str) -> Run {
    let run = if case.direction == "up" {
        let target = remote.guarded_folder("raw-up");
        run_job(
            job(
                Endpoint::Local,
                Endpoint::Remote(remote.backend.clone()),
                vec![slash(source)],
                target.path.clone(),
                case.name,
            ),
            CASE_LIMIT,
        )
    } else {
        let target = scratch.sub(&format!("down-{}", crate::support::unique("d")));
        run_job(
            job(
                Endpoint::Remote(remote.backend.clone()),
                Endpoint::Local,
                vec![file.to_string()],
                slash(&target),
                case.name,
            ),
            CASE_LIMIT,
        )
    };
    assert_eq!(run.bytes, RAW_BYTES, "{}: every byte arrives", case.name);
    run
}

/// Bytes per second of the plain client over the same file and server.
fn client_rate(
    case: &Case,
    remote: &Remote,
    source: &Path,
    scratch: &Scratch,
    file: &str,
) -> Result<f64, String> {
    let server = remote
        .server
        .as_ref()
        .ok_or("kein Server für den Vergleichsclient")?;
    let up = case.direction == "up";
    let landing = scratch.sub("client");
    let local = if up {
        source.to_path_buf()
    } else {
        landing.join("large.bin")
    };
    let folder = remote.guarded_folder("raw-client");
    let target = remote.join(&folder.path, "large.bin");
    let (program, args) = match remote.way {
        Way::Sftp => {
            let key = need("SE_SPEED_SFTP_KEY", "privater Schlüssel für sftp -b")?;
            let batch = scratch.path().join("batch.txt");
            let line = if up {
                format!("put \"{}\" \"{target}\"\n", local.display())
            } else {
                format!("get \"{file}\" \"{}\"\n", local.display())
            };
            std::fs::write(&batch, line).map_err(|error| error.to_string())?;
            let mut args = strings(&["-b", &batch.to_string_lossy(), "-i", &key, "-P"]);
            args.push(server.port.to_string());
            args.extend(strings(&[
                "-o",
                "IdentitiesOnly=yes",
                "-o",
                "StrictHostKeyChecking=no",
                "-o",
                "UserKnownHostsFile=/dev/null",
                "-o",
                "LogLevel=ERROR",
            ]));
            args.push(format!("{}@{}", server.user, server.host));
            ("sftp", args)
        }
        way => {
            let login = format!("{}:{}", server.user, server.password);
            let (host, port) = (&server.host, server.port);
            // FTP paths are absolute (`//` after the port); WebDAV paths are URL paths.
            let url = match (way, up) {
                (Way::Ftp, true) => format!("ftp://{host}:{port}/{}/", folder.path),
                (Way::Ftp, false) => format!("ftp://{host}:{port}/{file}"),
                (_, true) => format!("http://{host}:{port}{target}"),
                (_, false) => format!("http://{host}:{port}{file}"),
            };
            let mut args = strings(&["-sS", "--fail", "-u", &login]);
            if up {
                args.extend(strings(&["-T", &local.to_string_lossy()]));
            } else {
                args.extend(strings(&["-o", &local.to_string_lossy()]));
            }
            args.push(url);
            ("curl", args)
        }
    };
    let done = run_tool(program, &args, &[], CLIENT_LIMIT)?;
    if !done.ok() {
        return Err(format!("{program}: {}", done.describe()));
    }
    Ok(RAW_BYTES as f64 / done.wall.as_secs_f64().max(0.001))
}

pub fn run_raw(case: Case) -> Result<(), String> {
    check_profile(&case)?;
    let way = Way::parse(case.way);
    let remote = connect(way)?;
    let scratch = Scratch::new(&fast_dir()?, case.name);
    let source = scratch.path().join("large.bin");
    random_file(&source, RAW_BYTES);
    let seeded = remote.guarded_folder("raw-seed");
    let file = remote.join(&seeded.path, "large.bin");
    if case.direction == "down" {
        remote.seed_file(&seeded.path, &source, "large.bin")?;
        let _ = std::fs::remove_file(&source);
    }
    let tool = if way == Way::Sftp { "sftp -b" } else { "curl" };
    let other = client_rate(&case, &remote, &source, &scratch, &file)?;
    let trial = retry_once(|| {
        let run = se_run(&case, &remote, &source, &scratch, &file);
        let mut line = ReportLine::new(&case, "B/s");
        line.size_bytes = RAW_BYTES;
        line.files = run.files;
        line.se_rate = run.rate();
        line.against(other, 0.8, &format!("≥ 0,80 × Rate von {tool}"));
        line.reference_tool = tool.to_string();
        line.reference_rate = other;
        line.limiter = run.limiter.code().to_string();
        line.notes.push(format!(
            "parallel höchstens {}, Bereiche höchstens {}",
            run.max_parallel, run.max_segments
        ));
        let failure = shortfall(
            case.name,
            line.se_rate,
            other,
            0.8,
            tool,
            "gleiche Datei, gleicher Server",
            "B/s",
        );
        Trial { line, failure }
    });
    finish(&trial.line, trial.failure.as_deref());
    Ok(())
}

/// The SFTP default against the same server limited to one cipher: the
/// default must reach 0.95 of the faster of the two ciphers (U44-6 Punkt 8).
/// Each of the two cases checks its own cipher, so both together give the
/// comparison with the faster one.
pub fn run_cipher(case: Case) -> Result<(), String> {
    check_profile(&case)?;
    let cipher = case.name.trim_start_matches("speed_task_sftp_cipher_");
    let default = connect(Way::Sftp)?;
    let limited = connect_sftp_vars(
        "SE_SPEED_SFTP_CIPHER",
        "SE_SPEED_SFTP_CIPHER_FINGERPRINTS",
        "SE_SPEED_SFTP_CIPHER_CONTAINER",
    )?;
    let scratch = Scratch::new(&fast_dir()?, case.name);
    let source = scratch.path().join("large.bin");
    random_file(&source, RAW_BYTES);
    let up = Case::new(case.name, "sftp", "up", "raw");
    let trial = retry_once(|| {
        let theirs = se_run(&up, &limited, &source, &scratch, "");
        let ours = se_run(&up, &default, &source, &scratch, "");
        let mut line = ReportLine::new(&case, "B/s");
        line.size_bytes = RAW_BYTES;
        line.files = ours.files;
        line.se_rate = ours.rate();
        line.against(
            theirs.rate(),
            0.95,
            &format!("Voreinstellung ≥ 0,95 × Rate mit nur {cipher}"),
        );
        line.reference_tool = format!("SFTP-Server mit nur {cipher}");
        line.reference_rate = theirs.rate();
        line.limiter = ours.limiter.code().to_string();
        let failure = shortfall(
            case.name,
            line.se_rate,
            theirs.rate(),
            0.95,
            &format!("Rate mit nur {cipher}"),
            "gleiche Datei, gleiche Strecke",
            "B/s",
        );
        Trial { line, failure }
    });
    finish(&trial.line, trial.failure.as_deref());
    Ok(())
}

macro_rules! raw_cases {
    ($($name:ident => ($way:literal, $direction:literal)),* $(,)?) => {
        $(
            #[test]
            #[ignore = "needs the stage's server on the unshaped loopback"]
            fn $name() {
                let case = Case::new(stringify!($name), $way, $direction, "raw");
                guarded(case.name, || run_raw(case));
            }
        )*
    };
}

raw_cases! {
    speed_task_raw_sftp_large_up => ("sftp", "up"),
    speed_task_raw_sftp_large_down => ("sftp", "down"),
    speed_task_raw_ftp_large_up => ("ftp", "up"),
    speed_task_raw_ftp_large_down => ("ftp", "down"),
    speed_task_raw_webdav_large_up => ("webdav", "up"),
    speed_task_raw_webdav_large_down => ("webdav", "down"),
}

#[test]
#[ignore = "needs the stage's SFTP servers on the unshaped loopback"]
fn speed_task_sftp_cipher_aes_gcm() {
    let case = Case::new("speed_task_sftp_cipher_aes_gcm", "sftp", "up", "raw");
    guarded(case.name, || run_cipher(case));
}

#[test]
#[ignore = "needs the stage's SFTP servers on the unshaped loopback"]
fn speed_task_sftp_cipher_chacha() {
    let case = Case::new("speed_task_sftp_cipher_chacha", "sftp", "up", "raw");
    guarded(case.name, || run_cipher(case));
}
