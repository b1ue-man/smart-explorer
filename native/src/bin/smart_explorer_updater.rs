#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

#[path = "smart_explorer_updater/archive.rs"]
mod archive;
#[path = "smart_explorer_updater/args.rs"]
mod args;
#[path = "smart_explorer_updater/bookkeeping.rs"]
mod bookkeeping;
#[path = "smart_explorer_updater/hash.rs"]
mod hash;
#[path = "smart_explorer_updater/instance.rs"]
mod instance;
#[path = "smart_explorer_updater/launch.rs"]
mod launch;
#[path = "smart_explorer_updater/legacy.rs"]
mod legacy;
#[path = "smart_explorer_updater/legacy_intent.rs"]
mod legacy_intent;
#[path = "smart_explorer_updater/legacy_recovery.rs"]
mod legacy_recovery;
#[path = "smart_explorer_updater/logging.rs"]
mod logging;
#[path = "smart_explorer_updater/parent.rs"]
mod parent;
#[path = "smart_explorer_updater/path_safety.rs"]
mod path_safety;
#[path = "smart_explorer_updater/process.rs"]
mod process;
#[path = "smart_explorer_updater/replace.rs"]
mod replace;

use archive::archive_current_app;
use args::{ApplyArgs, ApplyRequest};
use bookkeeping::PreparedBookkeeping;
use hash::verify_sha256;
use launch::{spawn_verified_acknowledged, spawn_verified_detached};
use logging::{append_log, default_error_file, record_failure};
use process::{stop_target_processes_for_update, wait_for_pid_exit};
use replace::{replace_transaction_with_retries, AppliedTransaction, Replacement};
use std::time::Duration;

fn main() {
    let raw: Vec<String> = std::env::args().collect();
    if !args::has_key(&raw, "--apply") {
        return;
    }

    let request = match ApplyRequest::parse(&raw) {
        Ok(request) => request,
        Err(message) => {
            let _ = record_failure(&default_error_file(), &message);
            std::process::exit(1);
        }
    };
    let fallback_error_file = request.error_file().to_path_buf();
    let result = match request {
        ApplyRequest::Current(args) => apply_update(*args),
        ApplyRequest::Legacy(args) => legacy::apply_update(*args),
    };
    match result {
        Ok(()) => {}
        Err(failure) => {
            handle_failure_with(&fallback_error_file, &failure, |recovery| {
                spawn_verified_detached(
                    &recovery.executable,
                    &recovery.sha256,
                    &["--update-rollback"],
                )
            });
            std::process::exit(1);
        }
    }
}

#[derive(Debug)]
pub(crate) struct ApplyFailure {
    message: String,
    recovery: Option<RecoveryLaunch>,
}

#[derive(Debug)]
struct RecoveryLaunch {
    executable: std::path::PathBuf,
    sha256: String,
}

impl ApplyFailure {
    fn with_recovery(message: String, executable: &std::path::Path, sha256: &str) -> Self {
        Self {
            message,
            recovery: Some(RecoveryLaunch {
                executable: executable.to_path_buf(),
                sha256: sha256.to_string(),
            }),
        }
    }
}

impl From<String> for ApplyFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            recovery: None,
        }
    }
}

fn handle_failure_with(
    path: &std::path::Path,
    failure: &ApplyFailure,
    launch: impl FnOnce(&RecoveryLaunch) -> std::io::Result<()>,
) {
    let report_error = record_failure(path, &failure.message).err();
    if let Some(recovery) = &failure.recovery {
        if let Err(launch_error) = launch(recovery) {
            let message = format!(
                "{}; Wiederhergestellte Version {} konnte nicht gestartet werden: {launch_error}{}",
                failure.message,
                recovery.executable.display(),
                report_error
                    .as_deref()
                    .map(|error| format!(
                        "; vorheriger Fehlerstatus konnte nicht geschrieben werden: {error}"
                    ))
                    .unwrap_or_default()
            );
            let _ = record_failure(path, &message);
        }
    }
}

fn apply_update(args: ApplyArgs) -> Result<(), ApplyFailure> {
    append_log(&format!(
        "apply v{}: staged={} target={} parent={}",
        args.version,
        args.staged.display(),
        args.target.display(),
        args.parent_pid
    ));

    let helper = std::env::current_exe()
        .map_err(|error| format!("Updater-Helferpfad unbekannt: {error}"))?;
    validate_modern_paths(&args, &helper)?;
    verify_sha256(&helper, &args.helper_sha256)?;
    verify_sha256(&args.staged, &args.staged_sha256)?;
    verify_sha256(&args.cli_staged, &args.cli_sha256)?;

    wait_for_pid_exit(args.parent_pid, Duration::from_secs(300))?;
    let _instance = instance::acquire(&args.target)?;
    validate_modern_paths(&args, &helper)?;
    // The background daemon can be either the GUI executable or the standalone
    // terminal companion. Wait for both images before replacing either one;
    // retrying a locked CLI for only a few seconds is not a safe handoff.
    for target in [&args.target, &args.cli_target] {
        if let Err(e) = stop_target_processes_for_update(target) {
            if e.needs_elevation {
                return Err(elevation_refused("Prozessbereinigung").into());
            }
            return Err(e.msg.into());
        }
    }

    if let Err(error) = archive_current_app(&args.target, &args.target_sha256, &args.archive) {
        if error.needs_elevation {
            return Err(elevation_refused("Programmdatei archivieren").into());
        }
        return Err(format!("Aktuelle Programmdatei archivieren: {}", error.msg).into());
    }

    let replacements = [
        Replacement {
            label: "Updater-Helfer",
            staged: &helper,
            target: &args.helper_target,
            sha256: &args.helper_sha256,
            expected_target_sha256: None,
        },
        Replacement {
            label: "Terminal-Begleiter",
            staged: &args.cli_staged,
            target: &args.cli_target,
            sha256: &args.cli_sha256,
            expected_target_sha256: None,
        },
        Replacement {
            label: "Smart Explorer",
            staged: &args.staged,
            target: &args.target,
            sha256: &args.staged_sha256,
            expected_target_sha256: Some(&args.target_sha256),
        },
    ];
    let mut transaction = match replace_transaction_with_retries(&replacements) {
        Ok(transaction) => transaction,
        Err(error) if error.needs_elevation => {
            return Err(elevation_refused("Programmdateien ersetzen").into());
        }
        Err(error) => return Err(format!("Update-Transaktion: {}", error.msg).into()),
    };

    if let Err(error) = verify_sha256(&args.target, &args.staged_sha256) {
        return Err(restore_failed_update(
            &args,
            transaction,
            format!("Neue Programmdatei nach dem Ersetzen ungueltig: {error}"),
        ));
    }
    let mut bookkeeping = match PreparedBookkeeping::prepare(
        &args.last_applied,
        &args.error_file,
        &args.version,
        &[&args.manifest],
    ) {
        Ok(bookkeeping) => bookkeeping,
        Err(error) => {
            return Err(restore_failed_update(
                &args,
                transaction,
                format!("Update-Status vor dem Neustart vorbereiten: {error}"),
            ));
        }
    };
    if let Err(launch_error) =
        spawn_verified_acknowledged(&args.target, &args.staged_sha256, &["--updated"])
    {
        let status_error = bookkeeping.rollback().err();
        return Err(restore_failed_update(
            &args,
            transaction,
            format!(
                "Neue Version konnte nicht gestartet werden: {launch_error}{}",
                status_error
                    .as_deref()
                    .map(|error| format!("; Status-Rollback fehlgeschlagen: {error}"))
                    .unwrap_or_default()
            ),
        ));
    }

    transaction.finalize();
    finish_bookkeeping(&args, &helper, &mut bookkeeping);
    append_log(&format!("apply v{}: ok", args.version));
    Ok(())
}

fn elevation_refused(operation: &str) -> String {
    format!(
        "{operation} benoetigt Administratorrechte; der Updater startet Smart Explorer absichtlich nicht aus einem erhoehten Prozess, bitte den Installer verwenden"
    )
}

fn restore_failed_update(
    args: &ApplyArgs,
    mut transaction: AppliedTransaction,
    failure: String,
) -> ApplyFailure {
    let rollback_error = transaction.rollback().err();
    let candidate = match verified_previous_candidate(args, rollback_error.as_deref()) {
        Ok(candidate) => candidate,
        Err(fallback_error) => {
            return format!(
                "{failure}; Wiederherstellung der vorherigen Version fehlgeschlagen: {fallback_error}"
            )
            .into();
        }
    };
    let reported = format!(
        "{failure}; vorherige Version wurde wiederhergestellt und wird neu gestartet{}",
        rollback_error
            .as_deref()
            .map(|error| format!(" (Transaktions-Rollback meldete: {error})"))
            .unwrap_or_default()
    );
    ApplyFailure::with_recovery(reported, candidate, &args.target_sha256)
}

fn verified_previous_candidate<'a>(
    args: &'a ApplyArgs,
    rollback_error: Option<&str>,
) -> Result<&'a std::path::Path, String> {
    let candidate = if verify_sha256(&args.target, &args.target_sha256).is_ok() {
        &args.target
    } else {
        verify_sha256(&args.archive, &args.target_sha256).map_err(|archive_error| {
            format!(
                "Rollback: {}; Archiv unbrauchbar: {archive_error}",
                rollback_error.unwrap_or("Programmdatei nicht wiederhergestellt")
            )
        })?;
        &args.archive
    };
    verify_sha256(candidate, &args.target_sha256)?;
    Ok(candidate)
}

fn finish_bookkeeping(
    args: &ApplyArgs,
    helper: &std::path::Path,
    bookkeeping: &mut PreparedBookkeeping,
) {
    for warning in bookkeeping.commit() {
        append_log(&format!("warning: {warning}"));
    }
    // The rollback pin is cleared only after the verified new app was spawned.
    if let Err(error) = std::fs::remove_file(&args.pin_file) {
        if error.kind() != std::io::ErrorKind::NotFound {
            append_log(&format!(
                "warning: rollback pin {} entfernen: {error}",
                args.pin_file.display()
            ));
        }
    }
    for (label, path) in [
        ("staged app", args.staged.as_path()),
        ("staged CLI", args.cli_staged.as_path()),
        ("staged helper", helper),
    ] {
        if let Err(error) = std::fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                append_log(&format!(
                    "warning: remove {label} {}: {error}",
                    path.display()
                ));
            }
        }
    }
}

fn validate_modern_paths(args: &ApplyArgs, helper: &std::path::Path) -> Result<(), String> {
    path_safety::validate_distinct_paths(&[
        ("Programmziel", &args.target),
        ("gestagte App", &args.staged),
        ("laufender Helfer", helper),
        ("installierter Helfer", &args.helper_target),
        ("gestagte CLI", &args.cli_staged),
        ("installierte CLI", &args.cli_target),
        ("Archiv", &args.archive),
        ("Update-Status", &args.last_applied),
        ("Fehlerstatus", &args.error_file),
        ("Staging-Manifest", &args.manifest),
        ("Rollback-Pin", &args.pin_file),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prelaunch_report_is_never_recreated_after_relaunched_app_consumes_it() {
        let dir = tempfile::tempdir().unwrap();
        let error_file = dir.path().join("error.txt");
        let failure = ApplyFailure::with_recovery(
            "recovery report".to_string(),
            std::path::Path::new("old-app"),
            &"a".repeat(64),
        );

        handle_failure_with(&error_file, &failure, |_| {
            assert_eq!(
                std::fs::read_to_string(&error_file).unwrap(),
                "recovery report"
            );
            std::fs::remove_file(&error_file)?;
            Ok(())
        });

        assert!(!error_file.exists());
    }

    #[test]
    fn unreported_failure_is_persisted_once() {
        let dir = tempfile::tempdir().unwrap();
        let error_file = dir.path().join("error.txt");
        let failure = ApplyFailure::from("new failure".to_string());

        handle_failure_with(&error_file, &failure, |_| {
            panic!("plain failure must not launch recovery")
        });

        assert_eq!(std::fs::read_to_string(error_file).unwrap(), "new failure");
    }
}
