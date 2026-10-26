//! Execute validated installation removal while retaining ownership and retry.
use super::removal_inventory::{
    exists, inventory, load_removal, remaining_managed_files, write_removal_record, RemovalFile,
    RemovalRecord,
};
use crate::mcp_launcher::installation::verify_handle;
use crate::mcp_launcher::{os, Installation};
use std::path::Path;

/// The installer runs the installed CLI, never a runtime or profile-selected
/// path. Only this installation's program files are in the removal boundary.
pub(crate) fn retire_current_installation() -> Result<(), String> {
    if crate::mcp_launcher::current_runtime().is_some() {
        return Err("uninstall_requires_installed_cli".into());
    }
    let image = std::fs::canonicalize(std::env::current_exe().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let root = image.parent().ok_or("installation_root_missing")?;
    if image != root.join(os::CLI_NAME) {
        return Err("uninstall_requires_installed_cli".into());
    }
    os::uninstall_programs_idle(root)?;
    if !exists(&root.join("mcp"))? {
        return Ok(());
    }
    let record = load_removal(root)?;
    if record.is_none() && !exists(&root.join("mcp/current.v1.json"))? {
        if remaining_managed_files(root)? {
            return Err(
                "uninstall_mcp_manifest_missing: recover this installation before removal".into(),
            );
        }
        return Ok(());
    }
    let manifest = match &record {
        Some(record) => record.manifest.clone(),
        None => Installation::load(root)?.manifest,
    };
    let mut running = os::running_image()?;
    verify_handle(&mut running, &manifest.runtime_sha256)?;
    drop(running);
    retire_mcp(root, record)
}

pub(super) fn retire_mcp(root: &Path, record: Option<RemovalRecord>) -> Result<(), String> {
    let manifest = match &record {
        Some(record) => record.manifest.clone(),
        None => Installation::load(root)?.manifest,
    };
    let installation = Installation {
        root: root.into(),
        manifest,
    };
    // Missing locks are accepted only during a recorded removal, after every
    // executable was already removed. Never recreate a lock while it is used.
    let lock = |name: &str| {
        if !exists(&root.join("mcp").join(name))? && record.is_some() {
            return Ok(None);
        }
        installation.lock(name, true).map(Some)
    };
    let owner = lock("update-owner.lock")?;
    let control = lock("control.lock")?;
    let operations = lock("operations.lock")?;
    if record.is_none() && Installation::load(root)?.manifest != installation.manifest {
        return Err("uninstall_installation_changed".into());
    }
    if installation.state()?.is_some_and(|state| {
        state.blocks_work()
            || (state.retire_managed
                && state.phase == crate::mcp_launcher::core::lifecycle::Phase::Committed
                && installation.manifest == state.previous)
    }) {
        return Err("uninstall_requires_update_recovery".into());
    }
    let record = match record {
        Some(record) => record,
        None => inventory(&installation)?,
    };
    if owner.is_none() || control.is_none() || operations.is_none() {
        for file in &record.files {
            if (file.path.ends_with(&format!("/{}", os::CLI_NAME))
                || file.path.ends_with(&format!("/{}", os::LAUNCHER_NAME)))
                && exists(&root.join(&file.path))?
            {
                return Err("uninstall_missing_locks_with_executable_owners_unproved".into());
            }
        }
    }
    let mut lifetimes = Vec::new();
    for file in &record.files {
        let path = root.join(&file.path);
        if !exists(&path)? {
            continue;
        }
        verify_removal_file(root, file)?;
        if file.path.ends_with(&format!("/{}", os::CLI_NAME))
            || file.path.ends_with(&format!("/{}", os::LAUNCHER_NAME))
        {
            if !os::processes_for(&path)?.is_empty() {
                return Err(format!(
                    "uninstall_busy: {} still has program owners",
                    path.display()
                ));
            }
            if file.path.starts_with("mcp/runtimes/") {
                let lifetime = path
                    .parent()
                    .ok_or("runtime_parent_missing")?
                    .join("lifetime.lock");
                os::validate_tree(root, &lifetime)?;
                lifetimes.push(os::lock(&lifetime, true)?);
            }
        }
    }
    // This durable, validated inventory also blocks every new managed start.
    // A partial deletion is retried against these exact original hashes.
    write_removal_record(root, &record)?;
    for file in record
        .files
        .iter()
        .filter(|file| !file.path.ends_with(".lock"))
    {
        remove_proved_file(root, file)?;
    }
    drop(lifetimes);
    drop(operations);
    drop(control);
    drop(owner);
    for file in record
        .files
        .iter()
        .filter(|file| file.path.ends_with(".lock"))
    {
        remove_proved_file(root, file)?;
    }
    for directory in &record.directories {
        remove_empty(root, &root.join(directory))?;
    }
    std::fs::remove_file(root.join("mcp/uninstall.v1.json")).map_err(|e| e.to_string())?;
    remove_empty(root, &root.join("mcp"))
}

fn verify_removal_file(root: &Path, file: &RemovalFile) -> Result<(), String> {
    os::validate_tree(root, &root.join(&file.path))?;
    if crate::updater::file_sha256(&root.join(&file.path))? != file.sha256 {
        return Err(format!("uninstall_content_changed: {}", file.path));
    }
    Ok(())
}

fn remove_proved_file(root: &Path, file: &RemovalFile) -> Result<(), String> {
    if !exists(&root.join(&file.path))? {
        return Ok(());
    }
    verify_removal_file(root, file)?;
    std::fs::remove_file(root.join(&file.path)).map_err(|e| format!("{}: {e}", file.path))
}

fn remove_empty(root: &Path, path: &Path) -> Result<(), String> {
    if !exists(path)? {
        return Ok(());
    }
    os::validate_tree(root, path)?;
    match std::fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
