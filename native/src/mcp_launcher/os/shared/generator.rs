//! Bind embedded templates to the executable admitted at process entry.
use crate::mcp_launcher::{os, Installation, RuntimeContract, RuntimeIdentity};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};
struct EntryImage {
    root: PathBuf,
    path: PathBuf,
    sha256: String,
}
static ENTRY: OnceLock<Result<EntryImage, String>> = OnceLock::new();
pub(crate) fn capture_entry() {
    let selected = std::env::args_os()
        .nth(1)
        .is_some_and(|arg| matches!(arg.to_str(), Some("plugin" | "mcp" | "update")));
    if !selected {
        return;
    }
    ENTRY.get_or_init(|| {
        let path = std::fs::canonicalize(std::env::current_exe().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let root = std::fs::canonicalize(path.parent().ok_or("generator_parent_missing")?)
            .map_err(|e| e.to_string())?;
        let mut file = os::running_image()?;
        let mut digest = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        Ok(EntryImage {
            root,
            path,
            sha256: format!("{digest:x}", digest = digest.finalize()),
        })
    });
}
pub(crate) fn identity() -> Result<Option<RuntimeIdentity>, String> {
    let image = ENTRY
        .get()
        .ok_or("integration_generator_entry_missing")?
        .as_ref()
        .map_err(|error| error.clone())?;
    if !image.root.join("mcp/current.v1.json").exists() {
        return Ok(None);
    }
    let installation = Installation::load(&image.root)?;
    if image.path != installation.installed_cli()
        || image.sha256 != installation.manifest.runtime_sha256
    {
        return Err(
            "integration_generator_stale: run the currently installed se plugin refresh".into(),
        );
    }
    let contract = RuntimeContract {
        runtime_sha256: installation.manifest.runtime_sha256.clone(),
        ipc_contract_id: installation.manifest.ipc_contract_id.clone(),
        shared_state_contract_id: installation.manifest.shared_state_contract_id.clone(),
    };
    Ok(Some(RuntimeIdentity {
        executable: installation.runtime_path(),
        installation,
        contract,
    }))
}
/// Legacy terminal updates have no managed manifest. Their local CLI remains
/// usable offline, but an already replaced running generator must not publish
/// templates for a different image now occupying its pathname.
pub(crate) fn legacy_entry_path() -> Result<PathBuf, String> {
    let image = ENTRY
        .get()
        .ok_or("integration_generator_entry_missing")?
        .as_ref()
        .map_err(|error| error.clone())?;
    verified_legacy_entry(&image.path, &image.sha256)
}
pub(in crate::mcp_launcher) fn verified_legacy_entry(
    path: &Path,
    hash: &str,
) -> Result<PathBuf, String> {
    let root = path.parent().ok_or("generator_parent_missing")?;
    if root.join("mcp/uninstall.v1.json").exists() {
        return Err("installation_uninstall_in_progress".into());
    }
    if [
        "current.v1.json",
        "installation.id",
        "legacy.v1.json",
        "update.v1.json",
    ]
    .iter()
    .any(|name| root.join("mcp").join(name).exists())
    {
        return Err("integration_legacy_entry_has_managed_state: retry through the installation or recover its managed state".into());
    }
    let mut current = std::fs::File::open(path).map_err(|e| e.to_string())?;
    crate::mcp_launcher::installation::verify_handle(&mut current, hash)?;
    Ok(path.into())
}
pub(crate) fn legacy_profile_guard(
) -> Result<Option<crate::updater::LegacyIntegrationGuard>, String> {
    let image = ENTRY
        .get()
        .ok_or("integration_generator_entry_missing")?
        .as_ref()
        .map_err(|error| error.clone())?;
    if image.root.join("mcp/current.v1.json").exists() {
        return Ok(None);
    }
    verified_legacy_entry(&image.path, &image.sha256)?;
    let guard = crate::updater::lock_cli_for_integration(&image.path)?;
    verified_legacy_entry(&image.path, &image.sha256)?;
    guard.verify_current()?;
    Ok(Some(guard))
}
/// Always execute the confirmed installed code after Commit; never invoke old
/// templates in the updating process, even when its original path was replaced.
pub fn refresh_installed_profile(root: &Path) -> Result<(), String> {
    let installation = Installation::load(root)?;
    let path = installation.installed_cli();
    let image = os::open_verified(
        &installation.root,
        &path,
        &installation.manifest.runtime_sha256,
    )?;
    let mut command = Command::new(&path);
    command.args(["plugin", "refresh"]).env_remove("COMPLETE");
    let mut child = os::spawn_installation_probe(command, image)?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let output = std::thread::Builder::new()
        .name("se-profile-refresh-output".into())
        .spawn(move || {
            if let Some(mut pipe) = stdout {
                let _ = std::io::copy(&mut pipe, &mut std::io::sink());
            }
        })
        .map_err(|e| {
            let _ = child.kill();
            let _ = child.wait();
            e.to_string()
        })?;
    let errors = std::thread::Builder::new()
        .name("se-profile-refresh-errors".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = stderr {
                let _ = pipe.by_ref().take(16384).read_to_end(&mut bytes);
                let _ = std::io::copy(&mut pipe, &mut std::io::sink());
            }
            bytes
        })
        .map_err(|e| {
            let _ = child.kill();
            let _ = child.wait();
            e.to_string()
        })?;
    let status = child.wait().map_err(|e| e.to_string())?;
    let _ = output.join();
    let errors = errors.join().map_err(|_| "profile_refresh_reader_failed")?;
    if !status.success() {
        return Err(format!(
            "integration_refresh_failed: {}",
            String::from_utf8_lossy(&errors).trim()
        ));
    }
    Ok(())
}
pub fn refresh_entry_installation() -> Result<(), String> {
    let image = ENTRY
        .get()
        .ok_or("integration_generator_entry_missing")?
        .as_ref()
        .map_err(|error| error.clone())?;
    if !image.root.join("mcp/current.v1.json").exists() {
        return Ok(());
    }
    refresh_installed_profile(&image.root)
}
