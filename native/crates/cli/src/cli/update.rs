//! `se update`: check the configured update feed and install a newer version.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use clap::Args;
use serde::{Deserialize, Serialize};

use crate::daemon::WorkerHandoff;
use crate::updater::{FeedCheck, Installation, ReplacedCli, UnsignedConsent, UnsignedFeed};

/// The replaced `se` must answer within this time, including a worker handoff.
const COMPLETION_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_SOURCE_BYTES: usize = 4096;
const NO_FEED: &str = concat!(
    "no update feed is configured: set one with `se update --source <feed URL or folder>` ",
    "(the same setting as the app's UPDATE field) or reinstall with install-linux.sh"
);
const REINSTALL_DESKTOP: &str = concat!(
    "--reinstall applies to a terminal-only installation; next to the desktop app ",
    "se update installs newer versions through the app's updater"
);
const DESKTOP_NOTE: &str = concat!(
    "the updater replaces Smart Explorer, its updater and se after this command ",
    "exits and then starts Smart Explorer; close Smart Explorer if it is open"
);

const UPDATE_HELP: &str = "\
Checks the configured update feed (the same setting as the app's UPDATE field)
and installs a newer version.

Terminal-only installation (install-linux.sh --cli-only): se downloads the
feed's se, verifies its SHA-256 and replaces the installed file itself; a link
such as ~/.local/bin/se stays and keeps pointing to it. The copy is verified
again right before the swap and before it is started, and a hash-verified
backup is kept until then. If the new se does not start or reports another
version than the feed, or Ctrl+C interrupts the update, the previous file is
restored. A running background worker of the old version is then handed over to
the new version; one se update at a time may replace a file.

Next to the desktop app, se update stages app, updater and se exactly like the
app's update check and starts the hash-bound updater. It replaces all three
after this command exits and then starts Smart Explorer, so close the app first;
this needs a graphical session.

A signed release must verify. An unsigned release of the project's feed or an
unsigned http:// feed needs --allow-unsigned for this one update.

  se update --check --json
  se update
  se update --source https://github.com/OWNER/REPO
  se update --startup-check false --check";

#[derive(Args)]
#[command(long_about = UPDATE_HELP)]
pub(super) struct UpdateArgs {
    #[arg(
        long,
        help = "Only report current and available versions; change nothing"
    )]
    check: bool,
    #[arg(
        long,
        conflicts_with = "check",
        help = "Install the feed's version again although it is not newer (terminal-only)"
    )]
    reinstall: bool,
    #[arg(
        long,
        value_name = "URL|FOLDER",
        help = "Save this update feed first (the setting behind the app's UPDATE field)"
    )]
    source: Option<String>,
    #[arg(
        long,
        value_name = "true|false",
        help = "Save whether the desktop app looks for updates when it starts (the app's \"Beim Start nach Updates suchen\"); combine with --check to change nothing else"
    )]
    startup_check: Option<bool>,
    #[arg(
        long,
        conflicts_with = "check",
        help = "Install this one update although its release is not signed (a release published before signing, a plain http:// feed); asked again for every update"
    )]
    allow_unsigned: bool,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
    // Run by the previous `se` right after it replaced this file, with the
    // version the feed promised; every later `se` keeps this contract.
    #[arg(
        long,
        hide = true,
        value_name = "VERSION",
        conflicts_with_all = ["check", "reinstall", "source", "startup_check", "allow_unsigned"]
    )]
    complete_install: Option<String>,
}

/// What the freshly installed `se` reports back to the one it replaced.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct CompletedInstall {
    version: String,
    worker: Option<WorkerHandoff>,
    worker_error: Option<String>,
}

struct Installed {
    target: PathBuf,
    sha256: String,
    completed: CompletedInstall,
    unsigned: Option<UnsignedFeed>,
}

struct Report<'a> {
    check: &'a FeedCheck,
    installation: &'a Installation,
    previous_error: Option<String>,
}

pub(super) fn run(args: UpdateArgs) -> Result<i32, String> {
    if let Some(expected) = &args.complete_install {
        return complete_install(expected);
    }
    if let Some(enabled) = args.startup_check {
        crate::updater::set_startup_check(enabled)
            .map_err(|error| format!("save the start-up update check: {error}"))?;
    }
    if let Some(source) = &args.source {
        // Only a feed that answers replaces the source the app uses too.
        let source = validate_source(source)?;
        crate::updater::read_feed_version(source)
            .map_err(|error| format!("the update source {source} is not usable: {error}"))?;
        crate::updater::set_update_source(source)
            .map_err(|error| format!("save update source: {error}"))?;
    }
    let installation = crate::updater::detect_installation()?;
    let check = crate::updater::check_feed()?.ok_or_else(|| NO_FEED.to_string())?;
    let report = Report {
        check: &check,
        installation: &installation,
        previous_error: crate::updater::peek_updater_error(),
    };
    if args.check {
        let status = if check.update_available {
            "update_available"
        } else {
            "up_to_date"
        };
        print(&report, status, None, None, args.json)?;
        return Ok(0);
    }
    if !check.update_available && !args.reinstall {
        print(&report, "up_to_date", None, None, args.json)?;
        return Ok(0);
    }
    if crate::updater::is_newer(&check.current_version, &check.feed_version) {
        let (offered, running) = (&check.feed_version, &check.current_version);
        return Err(format!(
            "the feed offers {offered}, older than this se {running}; se update does not downgrade"
        ));
    }
    // The consent covers exactly the version this run installs (E12).
    let consent = match args.allow_unsigned {
        true => UnsignedConsent::for_version(&check.feed_version),
        false => UnsignedConsent::none(),
    };
    match &installation {
        Installation::TerminalOnly { cli } => {
            let installed = install_terminal_only(cli, &check.feed_version, &consent)?;
            let status = if check.update_available {
                "updated"
            } else {
                "reinstalled"
            };
            print(&report, status, Some(&installed), None, args.json)?;
        }
        Installation::Desktop { .. } => {
            if args.reinstall {
                return Err(REINSTALL_DESKTOP.to_string());
            }
            crate::updater::desktop_update_possible()?;
            let bundle = crate::updater::stage_update(&check.feed_version, &consent)?;
            crate::updater::apply_staged_update_for(&bundle, &installation)?;
            let signed = Some(bundle.unsigned().map_or("verified", UnsignedFeed::code));
            print(&report, "handed_to_updater", None, signed, args.json)?;
        }
    }
    Ok(0)
}

fn validate_source(source: &str) -> Result<&str, String> {
    let source = source.trim();
    let one_line = !source.chars().any(char::is_control);
    if source.is_empty() || source.len() > MAX_SOURCE_BYTES || !one_line {
        return Err("the update source must be one line: a feed URL or folder".to_string());
    }
    Ok(source)
}

/// Replaces the installed `se`, lets the new file prove that it starts and
/// hands the worker over, and restores the previous file if it does not.
fn install_terminal_only(
    cli: &Path,
    version: &str,
    consent: &UnsignedConsent,
) -> Result<Installed, String> {
    let replaced = crate::updater::replace_cli(cli, version, consent)?;
    let unsigned = replaced.unsigned();
    // The exact verified bytes, checked again right before they are started.
    if let Err(error) = replaced.verify_installed() {
        return Err(restore(replaced, error));
    }
    let completed = match run_completion(replaced.target(), version) {
        Ok(completed) if completed.version == version => completed,
        Ok(completed) => {
            let found = completed.version;
            let failure = format!("the new se reports {found} instead of {version}");
            return Err(restore(replaced, failure));
        }
        Err(error) => {
            let failure = format!("the new se did not start correctly: {error}");
            return Err(restore(replaced, failure));
        }
    };
    let target = replaced.target().to_path_buf();
    let sha256 = replaced.sha256().to_string();
    replaced.commit()?;
    crate::cli::plugin::refresh_generated_files();
    Ok(Installed {
        target,
        sha256,
        completed,
        unsigned,
    })
}

fn restore(replaced: ReplacedCli, failure: String) -> String {
    let backup = replaced.backup().display().to_string();
    match replaced.rollback() {
        Ok(()) => format!("{failure}; the previous se was restored"),
        Err(error) => format!(
            "{failure}; restoring the previous se failed: {error}; the previous se is {backup}"
        ),
    }
}

fn run_completion(target: &Path, version: &str) -> Result<CompletedInstall, String> {
    let mut child = Command::new(target)
        .args(["update", "--complete-install", version])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("start {}: {error}", target.display()))?;
    let deadline = Instant::now() + COMPLETION_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let limit = COMPLETION_TIMEOUT.as_secs();
            return Err(format!("no answer within {limit} seconds"));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let mut output = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        stdout
            .read_to_string(&mut output)
            .map_err(|error| error.to_string())?;
    }
    if !status.success() {
        return Err(format!("it exited with {status}"));
    }
    serde_json::from_str(output.trim())
        .map_err(|error| format!("unreadable answer {output:?}: {error}"))
}

/// Runs in the new `se`: reports the installed version and, only when it is
/// the promised one, hands a worker of the previous version over to it. A
/// mismatching file is rolled back by the caller and must not keep a worker.
fn complete_install(expected: &str) -> Result<i32, String> {
    let version = env!("CARGO_PKG_VERSION");
    let (worker, worker_error) = if version != expected {
        (
            None,
            Some(format!("not {expected}; the worker was left alone")),
        )
    } else {
        match crate::daemon::hand_off_running_worker() {
            Ok(handoff) => (Some(handoff), None),
            Err(error) => (None, Some(error)),
        }
    };
    let completed = CompletedInstall {
        version: version.to_string(),
        worker,
        worker_error,
    };
    let answer = serde_json::to_string(&completed).map_err(|error| error.to_string())?;
    println!("{answer}");
    Ok(0)
}

fn print(
    report: &Report<'_>,
    status: &str,
    installed: Option<&Installed>,
    staged_signature: Option<&str>,
    json: bool,
) -> Result<(), String> {
    let check = report.check;
    let note = (status == "handed_to_updater").then_some(DESKTOP_NOTE);
    // `None` when this run installed and staged nothing.
    let signature = installed
        .map(|installed| installed.unsigned.map_or("verified", UnsignedFeed::code))
        .or(staged_signature);
    if json {
        let installed = installed.map(|installed| {
            serde_json::json!({
                "path": installed.target,
                "sha256": installed.sha256,
                "worker": installed.completed.worker,
                "worker_error": installed.completed.worker_error,
            })
        });
        let value = serde_json::json!({
            "status": status,
            "current_version": check.current_version,
            "feed_version": check.feed_version,
            "update_available": check.update_available,
            "source": check.source,
            "installation": report.installation,
            "installed": installed,
            "note": note,
            "signature": signature,
            "previous_update_error": report.previous_error,
            // The app's "Auto-Update pausiert (zurückgerollt auf v…)".
            "auto_update_paused_at": crate::updater::pinned_version(),
            "startup_check": crate::updater::startup_check_enabled(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    println!("status\t{status}");
    println!("current_version\t{}", check.current_version);
    println!("feed_version\t{}", check.feed_version);
    println!("source\t{}", clean(&check.source));
    match report.installation {
        Installation::TerminalOnly { cli } => {
            println!("installation\tterminal_only\t{}", cli.display());
        }
        Installation::Desktop { cli, app } => {
            let (app, cli) = (app.display(), cli.display());
            println!("installation\tdesktop\t{app}\tse={cli}");
        }
    }
    if let Some(installed) = installed {
        let (path, sha256) = (installed.target.display(), &installed.sha256);
        println!("installed\t{path}\tsha256={sha256}");
        match (
            &installed.completed.worker,
            &installed.completed.worker_error,
        ) {
            (Some(handoff), _) => println!("worker\t{}", handoff_code(*handoff)),
            (None, Some(error)) => println!("worker\terror\t{}", clean(error)),
            (None, None) => println!("worker\tunknown"),
        }
    }
    if let Some(signature) = signature {
        println!("signature\t{signature}");
    }
    if let Some(note) = note {
        println!("note\t{note}");
    }
    if let Some(error) = &report.previous_error {
        println!("previous_update_error\t{}", clean(error));
    }
    if let Some(line) = paused_line(crate::updater::pinned_version().as_deref()) {
        println!("{line}");
    }
    println!(
        "startup_check\t{}",
        if crate::updater::startup_check_enabled() {
            "on"
        } else {
            "off"
        }
    );
    Ok(())
}

/// The app shows an automatic-update pause after a rollback; the terminal
/// names it the same way.
fn paused_line(pinned: Option<&str>) -> Option<String> {
    pinned.map(|version| format!("auto_update\tpaused after a rollback to {}", clean(version)))
}

fn handoff_code(handoff: WorkerHandoff) -> &'static str {
    match handoff {
        WorkerHandoff::NotRunning => "not_running",
        WorkerHandoff::Current => "current",
        WorkerHandoff::Replaced => "replaced",
    }
}

fn clean(value: &str) -> String {
    value.replace(['\t', '\r', '\n'], " ")
}

#[cfg(test)]
#[path = "update_tests.rs"]
mod tests;
