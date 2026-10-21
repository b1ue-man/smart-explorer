//! The terminal on the engine (M-6 "Terminal", U49): `se put`, `se get` and
//! `se cp -r` against the SFTP server over the `wan` link. The rate of the
//! process (wall time) must reach 0.9 of the engine case of the same data,
//! measured earlier in the same run (`speed_task_sftp_{large,small}_*_wan`,
//! read from `SE_SPEED_REPORT`: the engine cases come first in the stage
//! order). The last case checks that the old path keeps refusing an existing
//! destination.
use crate::data::{random_file, small_tree, Scratch};
use crate::remote::{connect, FolderGuard, Remote, Way};
use crate::support::{
    check_profile, fast_dir, finish, guarded, need, retry_once, shortfall, Case, ReportLine, Trial,
};
use crate::tools::{run_tool, strings, Exec};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

const CLI_LIMIT: Duration = Duration::from_secs(900);
const FOLDERS: usize = 100;

/// The last measured report line of an engine case.
fn engine_line(name: &str) -> Result<Value, String> {
    let report = need("SE_SPEED_REPORT", "Berichtsdatei der Stufe")?;
    let text = std::fs::read_to_string(&report).map_err(|error| format!("{report}: {error}"))?;
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .rfind(|value| {
            value["name"] == name
                && value["status"] == "measured"
                && value["skipped"].as_array().is_none_or(|skipped| skipped.is_empty())
        })
        .ok_or_else(|| {
            format!("der Engine-Fall {name} hat keine Messzeile im Bericht (Stufe speed-net-large oder speed-net-small)")
        })
}

/// `se` with an isolated profile holding the saved SFTP connection the stage
/// prepared (`SE_SPEED_CLI_ROOT`, label `SE_SPEED_CLI_LABEL`).
struct Se {
    binary: String,
    envs: Vec<(String, String)>,
    label: String,
}

impl Se {
    fn new() -> Result<Se, String> {
        let root = need("SE_SPEED_CLI_ROOT", "Profilordner des Terminal-Clients")?;
        let at = |sub: &str| format!("{root}/{sub}");
        let pairs = [
            ("HOME", at("home")),
            ("USERPROFILE", at("home")),
            ("XDG_DATA_HOME", at("data")),
            ("XDG_CONFIG_HOME", at("config")),
            ("XDG_RUNTIME_DIR", at("runtime")),
            ("APPDATA", at("data")),
            ("LOCALAPPDATA", at("data")),
        ];
        Ok(Se {
            binary: need("SE_SPEED_SE_BIN", "se des Kandidaten")?,
            envs: pairs
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
            label: need("SE_SPEED_CLI_LABEL", "Name der gespeicherten Verbindung")?,
        })
    }

    /// `@label:/rel` for a folder below the server root.
    fn target(&self, remote: &Remote, folder: &str, name: &str) -> String {
        let rel = remote.rel(folder);
        format!("@{}:/{rel}/{name}", self.label)
    }

    fn run(&self, args: &[&str]) -> Result<Exec, String> {
        run_tool(&self.binary, &strings(args), &self.envs, CLI_LIMIT)
    }
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

/// One CLI case: `build` makes the arguments of one run from a fresh
/// destination name, `unit` and `amount` turn the wall time into a rate.
struct Spec {
    engine: &'static str,
    unit: &'static str,
    amount: f64,
}

fn compare<'a>(
    case: &Case,
    se: &Se,
    spec: &Spec,
    mut command: impl FnMut() -> (Vec<String>, Option<FolderGuard<'a>>),
) -> Result<(), String> {
    let engine = engine_line(spec.engine)?;
    let reference = engine["se_rate"].as_f64().unwrap_or(0.0);
    let trial = retry_once(|| {
        // The guard keeps the destination folder on the server until the run is over.
        let (args, _guard) = command();
        let shown: Vec<&str> = args.iter().map(String::as_str).collect();
        let done = se
            .run(&shown)
            .unwrap_or_else(|reason| panic!("{}: {reason}", case.name));
        assert!(
            done.ok(),
            "{}: se {} -> {}",
            case.name,
            shown.join(" "),
            done.describe()
        );
        let rate = spec.amount / done.wall.as_secs_f64().max(0.001);
        let mut line = ReportLine::new(case, spec.unit);
        line.se_rate = rate;
        line.against(reference, 0.9, "≥ 0,90 × Rate des Engine-Falls");
        line.reference_tool = format!("Engine-Fall {}", spec.engine);
        line.reference_rate = reference;
        line.size_bytes = engine["size_bytes"].as_u64().unwrap_or(0);
        line.files = engine["files"].as_u64().unwrap_or(0);
        line.limiter = engine["limiter"].as_str().unwrap_or("").to_string();
        line.notes.push(format!(
            "se {} in {:.1} s; stderr: {}",
            shown.first().copied().unwrap_or(""),
            done.wall.as_secs_f64(),
            crate::support::one_line(&done.stderr.chars().take(300).collect::<String>())
        ));
        let failure = shortfall(
            case.name,
            rate,
            reference,
            0.9,
            "Rate des Engine-Falls",
            spec.engine,
            spec.unit,
        );
        Trial { line, failure }
    });
    finish(&trial.line, trial.failure.as_deref());
    Ok(())
}

fn large(case: Case, up: bool) -> Result<(), String> {
    check_profile(&case)?;
    let engine = if up {
        "speed_task_sftp_large_up_wan"
    } else {
        "speed_task_sftp_large_down_wan"
    };
    let bytes = engine_line(engine)?["size_bytes"].as_u64().unwrap_or(0);
    let se = Se::new()?;
    let remote = connect(Way::Sftp)?;
    let scratch = Scratch::new(&fast_dir()?, case.name);
    let source = scratch.path().join("large.bin");
    random_file(&source, bytes);
    let seeded = remote.guarded_folder("cli-seed");
    if !up {
        remote.seed_file(&seeded.path, &source, "large.bin")?;
        let _ = std::fs::remove_file(&source);
    }
    let spec = Spec {
        engine,
        unit: "B/s",
        amount: bytes as f64,
    };
    compare(&case, &se, &spec, || {
        if up {
            let folder = remote.guarded_folder("cli");
            let target = se.target(&remote, &folder.path, "large.bin");
            (
                strings(&["put", "--stats", &path_text(&source), &target]),
                Some(folder),
            )
        } else {
            let origin = se.target(&remote, &seeded.path, "large.bin");
            let landing = scratch.sub(&format!("down-{}", crate::support::unique("d")));
            (
                strings(&[
                    "get",
                    "--stats",
                    &origin,
                    &path_text(&landing.join("large.bin")),
                ]),
                None,
            )
        }
    })
}

fn small(case: Case, up: bool) -> Result<(), String> {
    check_profile(&case)?;
    let engine = if up {
        "speed_task_sftp_small_up_wan"
    } else {
        "speed_task_sftp_small_down_wan"
    };
    let files = engine_line(engine)?["files"].as_u64().unwrap_or(0);
    let se = Se::new()?;
    let remote = connect(Way::Sftp)?;
    let scratch = Scratch::new(&fast_dir()?, case.name);
    let tree = small_tree(&scratch.sub("tree"), FOLDERS, files as usize, 4096);
    let seeded = remote.guarded_folder("cli-seed");
    if !up {
        let place = remote.join(&seeded.path, "tree");
        remote
            .backend
            .mkdir_all(&place)
            .map_err(|error| format!("Ordner auf dem Server anlegen: {error}"))?;
        remote.seed_tree(&place, &tree)?;
    }
    let spec = Spec {
        engine,
        unit: "files/s",
        amount: files as f64,
    };
    compare(&case, &se, &spec, || {
        if up {
            let folder = remote.guarded_folder("cli");
            let target = se.target(&remote, &folder.path, "tree");
            (
                strings(&["cp", "-r", "--stats", &path_text(&tree), &target]),
                Some(folder),
            )
        } else {
            let origin = se.target(&remote, &seeded.path, "tree");
            let landing = scratch.sub(&format!("down-{}", crate::support::unique("d")));
            (
                strings(&[
                    "cp",
                    "-r",
                    "--stats",
                    &origin,
                    &path_text(&landing.join("tree")),
                ]),
                None,
            )
        }
    })
}

/// `se cp` onto an existing destination without `-f` stays refused (the old
/// path's behavior): exit code 1, a message that names `--force`, destination
/// unchanged.
fn refuses_existing() -> Result<(), String> {
    let name = "speed_task_cli_cp_refuses_existing_destination_unchanged";
    let se = Se::new()?;
    let scratch = Scratch::new(&fast_dir()?, name);
    let (source, destination) = (scratch.path().join("a.txt"), scratch.path().join("b.txt"));
    std::fs::write(&source, b"new content").map_err(|error| error.to_string())?;
    std::fs::write(&destination, b"old content").map_err(|error| error.to_string())?;
    let done = se.run(&["cp", &path_text(&source), &path_text(&destination)])?;
    let kept = std::fs::read(&destination).unwrap_or_default() == b"old content";
    let message_names_force = done.stderr.contains("--force");
    let case = Case::new(name, "local", "cp", "-");
    let mut line = ReportLine::new(&case, "B/s");
    line.requirement = "Exitcode ≠ 0, Meldung nennt --force, Ziel unverändert".into();
    line.notes.push(format!(
        "Exitcode {:?}; stderr: {}; Ziel unverändert: {kept}",
        done.code,
        crate::support::one_line(&done.stderr.chars().take(300).collect::<String>())
    ));
    let failure = (done.ok() || !message_names_force || !kept).then(|| {
        format!(
            "{name}: Exitcode {:?}, Meldung nennt --force: {message_names_force}, Ziel unverändert: {kept}",
            done.code
        )
    });
    finish(&line, failure.as_deref());
    Ok(())
}

#[test]
#[ignore = "needs se, the SFTP server, its saved connection and the engine cases' report"]
fn speed_task_cli_put_large_wan() {
    let case = Case::new("speed_task_cli_put_large_wan", "sftp", "up", "wan");
    guarded(case.name, || large(case, true));
}

#[test]
#[ignore = "needs se, the SFTP server, its saved connection and the engine cases' report"]
fn speed_task_cli_get_large_wan() {
    let case = Case::new("speed_task_cli_get_large_wan", "sftp", "down", "wan");
    guarded(case.name, || large(case, false));
}

#[test]
#[ignore = "needs se, the SFTP server, its saved connection and the engine cases' report"]
fn speed_task_cli_cp_small_tree_up_wan() {
    let case = Case::new("speed_task_cli_cp_small_tree_up_wan", "sftp", "up", "wan");
    guarded(case.name, || small(case, true));
}

#[test]
#[ignore = "needs se, the SFTP server, its saved connection and the engine cases' report"]
fn speed_task_cli_cp_small_tree_down_wan() {
    let case = Case::new(
        "speed_task_cli_cp_small_tree_down_wan",
        "sftp",
        "down",
        "wan",
    );
    guarded(case.name, || small(case, false));
}

#[test]
#[ignore = "needs se of the candidate"]
fn speed_task_cli_cp_refuses_existing_destination_unchanged() {
    guarded(
        "speed_task_cli_cp_refuses_existing_destination_unchanged",
        refuses_existing,
    );
}
