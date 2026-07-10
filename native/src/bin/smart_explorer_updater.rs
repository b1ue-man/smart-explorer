#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

#[path = "smart_explorer_updater/archive.rs"]
mod archive;
#[path = "smart_explorer_updater/args.rs"]
mod args;
#[path = "smart_explorer_updater/hash.rs"]
mod hash;
#[path = "smart_explorer_updater/launch.rs"]
mod launch;
#[path = "smart_explorer_updater/logging.rs"]
mod logging;
#[path = "smart_explorer_updater/process.rs"]
mod process;
#[path = "smart_explorer_updater/replace.rs"]
mod replace;

use archive::archive_current_app;
use args::{arg_value, ApplyArgs};
use hash::verify_sha256;
use launch::{relaunch_elevated, spawn_verified_detached};
use logging::{append_log, default_error_file, record_failure};
use process::{stop_target_processes_for_update, wait_for_pid_exit};
use replace::{replace_transaction, AppliedTransaction, ReplaceTargetError, Replacement};
use std::path::PathBuf;
use std::time::Duration;

fn main() {
    let raw: Vec<String> = std::env::args().collect();
    if !raw.iter().any(|a| a == "--apply") {
        return;
    }

    let fallback_error_file = arg_value(&raw, "--error-file")
        .map(PathBuf::from)
        .unwrap_or_else(default_error_file);
    match ApplyArgs::parse(&raw).and_then(apply_update) {
        Ok(()) => {}
        Err(e) => {
            record_failure(&fallback_error_file, &e);
            std::process::exit(1);
        }
    }
}

fn apply_update(args: ApplyArgs) -> Result<(), String> {
    append_log(&format!(
        "apply v{}: staged={} target={} parent={}",
        args.version,
        args.staged.display(),
        args.target.display(),
        args.parent_pid
    ));

    let helper = std::env::current_exe()
        .map_err(|error| format!("Updater-Helferpfad unbekannt: {error}"))?;
    verify_sha256(&helper, &args.helper_sha256)?;
    verify_sha256(&args.staged, &args.staged_sha256)?;
    verify_sha256(&args.cli_staged, &args.cli_sha256)?;

    wait_for_pid_exit(args.parent_pid, Duration::from_secs(300))?;
    if let Err(e) = stop_target_processes_for_update(&args.target) {
        if !args.elevated && e.needs_elevation {
            append_log("process cleanup needs elevation; relaunching updater with UAC");
            relaunch_elevated(&args)
                .map_err(|e| format!("Administratorfreigabe starten: {}", e))?;
            return Ok(());
        }
        return Err(e.msg);
    }

    if let Err(error) = archive_current_app(&args.target, &args.target_sha256, &args.archive) {
        if !args.elevated && error.needs_elevation {
            append_log("archive needs elevation; relaunching updater with UAC");
            relaunch_elevated(&args)
                .map_err(|error| format!("Administratorfreigabe starten: {error}"))?;
            return Ok(());
        }
        return Err(format!("Aktuelle Programmdatei archivieren: {}", error.msg));
    }

    let replacements = [
        Replacement {
            label: "Updater-Helfer",
            staged: &helper,
            target: &args.helper_target,
            sha256: &args.helper_sha256,
        },
        Replacement {
            label: "Terminal-Begleiter",
            staged: &args.cli_staged,
            target: &args.cli_target,
            sha256: &args.cli_sha256,
        },
        Replacement {
            label: "Smart Explorer",
            staged: &args.staged,
            target: &args.target,
            sha256: &args.staged_sha256,
        },
    ];
    let mut transaction = match replace_with_retries(&replacements) {
        Ok(transaction) => transaction,
        Err(error) if !args.elevated && error.needs_elevation => {
            append_log("replacement transaction needs elevation; relaunching updater with UAC");
            relaunch_elevated(&args)
                .map_err(|error| format!("Administratorfreigabe starten: {error}"))?;
            return Ok(());
        }
        Err(error) => return Err(format!("Update-Transaktion: {}", error.msg)),
    };

    verify_sha256(&args.target, &args.staged_sha256)?;
    if let Err(launch_error) =
        spawn_verified_detached(&args.target, &args.staged_sha256, &["--updated"])
    {
        let rollback_error = transaction.rollback().err();
        record_failure(
            &args.error_file,
            &format!(
                "Neue Version konnte nicht gestartet werden ({launch_error}); Rollback wird versucht{}",
                rollback_error
                    .as_deref()
                    .map(|error| format!(": {error}"))
                    .unwrap_or_default()
            ),
        );
        let fallback = relaunch_verified_previous(&args, rollback_error.as_deref());
        return Err(match fallback {
            Ok(()) => format!(
                "Neue Version konnte nicht gestartet werden ({launch_error}); vorherige Version wurde wiederhergestellt und gestartet"
            ),
            Err(fallback_error) => format!(
                "Neue Version konnte nicht gestartet werden ({launch_error}); Wiederherstellung/Neustart der vorherigen Version fehlgeschlagen: {fallback_error}"
            ),
        });
    }

    transaction.finalize();
    best_effort_bookkeeping(&args, &helper);
    append_log(&format!("apply v{}: ok", args.version));
    Ok(())
}

fn replace_with_retries(
    replacements: &[Replacement<'_>],
) -> Result<AppliedTransaction, ReplaceTargetError> {
    let mut last = None;
    for _ in 0..10 {
        match replace_transaction(replacements) {
            Ok(transaction) => return Ok(transaction),
            Err(error)
                if error.needs_elevation
                    || error.msg.contains("Rollback fehlgeschlagen")
                    || error.msg.contains("Pruefsumme") =>
            {
                return Err(error);
            }
            Err(error) => last = Some(error),
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    Err(last.unwrap_or_else(|| ReplaceTargetError::new("unbekannter Fehler", false)))
}

fn relaunch_verified_previous(
    args: &ApplyArgs,
    rollback_error: Option<&str>,
) -> Result<(), String> {
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
    spawn_verified_detached(candidate, &args.target_sha256, &["--update-rollback"])
        .map_err(|error| format!("{} starten: {error}", candidate.display()))
}

fn best_effort_bookkeeping(args: &ApplyArgs, helper: &std::path::Path) {
    let _ = std::fs::remove_file(&args.error_file);
    let mut warnings = Vec::new();
    if let Some(parent) = args.last_applied.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            warnings.push(format!("Update-Statusordner: {error}"));
        }
    }
    if let Err(error) = std::fs::write(&args.last_applied, &args.version) {
        warnings.push(format!("Update-Status: {error}"));
    }
    for (label, path) in [
        ("staging manifest", args.manifest.as_path()),
        ("rollback pin", args.pin_file.as_path()),
    ] {
        if let Err(error) = std::fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                warnings.push(format!("{label} {} entfernen: {error}", path.display()));
            }
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
    if !warnings.is_empty() {
        let message = format!(
            "Update v{} wurde gestartet, aber die Nachbereitung war unvollstaendig: {}",
            args.version,
            warnings.join("; ")
        );
        record_failure(&args.error_file, &message);
    }
}
