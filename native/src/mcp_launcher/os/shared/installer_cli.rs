//! Explicit installer provisioning and stable program-removal entry points.
use crate::mcp_launcher::{GenerationInput, Installation};
use std::ffi::OsString;
use std::path::PathBuf;

#[path = "installer_cli_removal.rs"]
mod removal;
#[path = "installer_cli_inventory.rs"]
mod removal_inventory;
pub(crate) use removal::retire_current_installation;

#[cfg(all(test, any(windows, target_os = "linux")))]
use crate::mcp_launcher::installation::{hash_bytes, read_json};
#[cfg(all(test, any(windows, target_os = "linux")))]
use crate::mcp_launcher::{os, Manifest};
#[cfg(all(test, any(windows, target_os = "linux")))]
use removal::retire_mcp;
#[cfg(all(test, any(windows, target_os = "linux")))]
use removal_inventory::{
    inventory, load_removal, remaining_managed_files, write_removal_record, RemovalFile,
};

#[cfg(all(test, any(windows, target_os = "linux")))]
#[path = "batches_uninstall_task_tests.rs"]
mod batches_uninstall_task_tests;

pub(crate) fn provision(arguments: &[OsString]) -> Result<i32, String> {
    let allowed = [
        "--root",
        "--runtime-source",
        "--runtime-sha256",
        "--launcher-source",
        "--launcher-sha256",
        "--version",
    ];
    let mut values = std::collections::BTreeMap::new();
    for pair in arguments.chunks(2) {
        if pair.len() != 2 {
            return Err("provision argument value missing".into());
        }
        let key = pair[0].to_str().ok_or("provision argument invalid")?;
        if !allowed.contains(&key) || values.insert(key, pair[1].clone()).is_some() {
            return Err(format!("provision unknown or duplicate argument: {key}"));
        }
    }
    let get = |key: &str| {
        values
            .get(key)
            .cloned()
            .ok_or_else(|| format!("provision required argument missing: {key}"))
    };
    let root = std::fs::canonicalize(PathBuf::from(get("--root")?)).map_err(|e| e.to_string())?;
    if root.join("mcp/uninstall.v1.json").exists() {
        return Err("installation_uninstall_in_progress".into());
    }
    let installation_id = match std::fs::read_to_string(root.join("mcp/installation.id")) {
        Ok(id) => id,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            crate::mcp_launcher::new_installation_id()?
        }
        Err(e) => return Err(e.to_string()),
    };
    let text = |key: &str| {
        get(key)?
            .into_string()
            .map_err(|_| format!("provision argument must be UTF-8: {key}"))
    };
    let input = GenerationInput {
        installation_id,
        product_version: text("--version")?,
        runtime_path: PathBuf::from(get("--runtime-source")?),
        runtime_sha256: text("--runtime-sha256")?,
        launcher_path: PathBuf::from(get("--launcher-source")?),
        launcher_sha256: text("--launcher-sha256")?,
    };
    if root.join("mcp/current.v1.json").exists() {
        let installed = Installation::load(&root)?;
        if installed.manifest.runtime_sha256 == input.runtime_sha256
            && installed.manifest.launcher_sha256 == input.launcher_sha256
            && installed.manifest.product_version == input.product_version
        {
            installed.checked_launcher_path()?;
            crate::mcp_launcher::verify_installed_product(
                &root,
                &installed.installed_cli(),
                &input.runtime_sha256,
                &input.product_version,
            )?;
            crate::mcp_launcher::contract_probe::verify_runtime(
                &root,
                &installed.runtime_path(),
                &input.runtime_sha256,
                &input.product_version,
            )?;
            return Ok(0);
        }
        return Err("existing installation requires the verified updater apply transaction".into());
    }
    let manifest = crate::mcp_launcher::stage_generation(&root, &input)?;
    crate::mcp_launcher::commit_initial_generation(&root, &manifest)?;
    Ok(0)
}
