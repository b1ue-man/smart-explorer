#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

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

use args::{arg_value, ApplyArgs};
use hash::verify_sha256;
use launch::{relaunch_elevated, spawn_detached};
use logging::{append_log, default_error_file, record_failure};
use process::{stop_target_processes_for_update, wait_for_pid_exit};
use replace::replace_target_from_staged;
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

    if let Some(expected) = args.helper_sha256.as_deref() {
        let helper =
            std::env::current_exe().map_err(|e| format!("Updater-Helferpfad unbekannt: {e}"))?;
        verify_sha256(&helper, expected)?;
    }

    let staged_len = std::fs::metadata(&args.staged)
        .map_err(|e| {
            format!(
                "gestagte Update-Datei fehlt ({}): {}",
                args.staged.display(),
                e
            )
        })?
        .len();

    if !wait_for_pid_exit(args.parent_pid, Duration::from_secs(30)) {
        append_log("parent did not exit within timeout; continuing with process cleanup");
    }
    if let Err(e) = stop_target_processes_for_update(&args.target) {
        if !args.elevated && e.needs_elevation {
            append_log("process cleanup needs elevation; relaunching updater with UAC");
            relaunch_elevated(&args)
                .map_err(|e| format!("Administratorfreigabe starten: {}", e))?;
            return Ok(());
        }
        return Err(e.msg);
    }

    let mut last_err = None;
    let mut last_needs_elevation = false;
    let mut replaced = false;
    for _ in 0..180 {
        match replace_target_from_staged(
            &args.staged,
            &args.target,
            staged_len,
            args.staged_sha256.as_deref(),
        ) {
            Ok(()) => {
                replaced = true;
                break;
            }
            Err(e) => {
                last_needs_elevation |= e.needs_elevation;
                last_err = Some(e.msg);
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    if !replaced {
        let msg = format!(
            "Smart Explorer konnte nicht ersetzt werden: {}",
            last_err.unwrap_or_else(|| "unbekannter Fehler".to_string())
        );
        if !args.elevated && last_needs_elevation {
            append_log("copy needs elevation; relaunching updater with UAC");
            relaunch_elevated(&args)
                .map_err(|e| format!("Administratorfreigabe starten: {}", e))?;
            return Ok(());
        }
        return Err(msg);
    }

    if let Some(parent) = args.last_applied.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&args.last_applied, &args.version)
        .map_err(|e| format!("Update-Status schreiben: {}", e))?;

    let _ = std::fs::remove_file(&args.error_file);
    let _ = std::fs::remove_file(&args.staged);
    spawn_detached(&args.target, &["--updated"])
        .map_err(|e| format!("Smart Explorer neu starten: {}", e))?;
    append_log(&format!("apply v{}: ok", args.version));
    Ok(())
}
