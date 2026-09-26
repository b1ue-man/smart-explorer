use std::path::{Path, PathBuf};

use super::archive::{exe_stem, pin_path, versions_dir};
use super::config::{last_applied_path, updater_error_path};
use super::core::{sha256_file, verify_sha256};
use super::os;
use super::staging::{manifest_matches, manifest_path, verify_staged_update};
use super::terminal::Installation;
use super::types::StagedUpdate;

/// Launch the staged, hash-bound helper after explicit user consent. This does
/// not replace files itself; the helper first waits for this process to exit.
pub fn apply_staged_update(bundle: &StagedUpdate) -> Result<(), String> {
    let target =
        std::env::current_exe().map_err(|error| format!("Eigener Pfad unbekannt: {error}"))?;
    let helper_target = os::installed_updater_path()?;
    let cli_target = os::installed_cli_path()?;
    launch_helper(bundle, &target, &helper_target, &cli_target)
}

/// `se update` beside the desktop app: the same helper, bound to the app
/// installed next to `se` instead of the running process.
pub fn apply_staged_update_for(
    bundle: &StagedUpdate,
    installation: &Installation,
) -> Result<(), String> {
    let Installation::Desktop { cli, app } = installation else {
        return Err("Der Updater-Helfer ersetzt nur eine Desktop-Installation".to_string());
    };
    let dir = cli
        .parent()
        .ok_or_else(|| format!("Installationsordner unbekannt: {}", cli.display()))?;
    launch_helper(bundle, app, &dir.join(os::installed_updater_name()), cli)
}

fn launch_helper(
    bundle: &StagedUpdate,
    target: &Path,
    helper_target: &Path,
    cli_target: &Path,
) -> Result<(), String> {
    manifest_matches(bundle)?;
    verify_staged_update(bundle)?;

    let target_sha256 = sha256_file(target)?;
    let archive = archive_path(target)?;

    let args = vec![
        "--apply".to_string(),
        "--target".to_string(),
        path_arg(target),
        "--target-sha256".to_string(),
        target_sha256,
        "--staged".to_string(),
        path_arg(bundle.app().path()),
        "--staged-sha256".to_string(),
        bundle.app().sha256().to_string(),
        "--helper-target".to_string(),
        path_arg(helper_target),
        "--helper-sha256".to_string(),
        bundle.helper().sha256().to_string(),
        "--cli-staged".to_string(),
        path_arg(bundle.cli().path()),
        "--cli-target".to_string(),
        path_arg(cli_target),
        "--cli-sha256".to_string(),
        bundle.cli().sha256().to_string(),
        "--archive".to_string(),
        path_arg(&archive),
        "--parent-pid".to_string(),
        std::process::id().to_string(),
        "--version".to_string(),
        bundle.version().to_string(),
        "--last-applied".to_string(),
        path_arg(&last_applied_path()),
        "--error-file".to_string(),
        path_arg(&updater_error_path()),
        "--manifest".to_string(),
        path_arg(&manifest_path()),
        "--pin-file".to_string(),
        path_arg(&pin_path()),
    ];

    // Keep this immediately adjacent to the process boundary. The helper also
    // validates itself on entry and fails closed if elevation would be needed.
    verify_sha256(bundle.helper().path(), bundle.helper().sha256())?;
    os::spawn_update_helper(bundle.helper().path(), bundle.helper().sha256(), &args)
}

fn archive_path(target: &std::path::Path) -> Result<PathBuf, String> {
    let dir = versions_dir().ok_or_else(|| "Versionsordner unbekannt".to_string())?;
    Ok(dir.join(format!(
        "{} {}{}",
        exe_stem(target),
        env!("CARGO_PKG_VERSION"),
        os::binary_suffix()
    )))
}

fn path_arg(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}
