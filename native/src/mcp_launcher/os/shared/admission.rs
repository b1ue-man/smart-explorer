use crate::mcp_launcher::{os, Installation, RuntimeIdentity};
use std::fs::File;
use std::sync::{Mutex, OnceLock};

static RUNTIME: OnceLock<Option<RuntimeIdentity>> = OnceLock::new();
static STARTER: OnceLock<Option<serde_json::Value>> = OnceLock::new();
static MIGRATION_ENTRY: OnceLock<bool> = OnceLock::new();
pub fn migration_entry() -> bool {
    MIGRATION_ENTRY.get().copied().unwrap_or(false)
}
pub fn current_starter() -> Option<serde_json::Value> {
    STARTER.get().cloned().flatten()
}
pub(crate) static LIFECYCLE: Mutex<Option<File>> = Mutex::new(None);

pub struct ProcessAdmission {
    _lifetime: Option<File>,
    _operation: Option<OperationGuard>,
}
pub struct OperationGuard {
    _lock: File,
    _lifetime: File,
}

pub fn current_runtime() -> Option<RuntimeIdentity> {
    RUNTIME.get().cloned().flatten()
}
/// Keep the returned guard for the entire CLI process. Independently opened
/// child locks are acknowledged before the parent's admission is released.
pub fn admit_cli_process() -> Result<ProcessAdmission, String> {
    crate::mcp_launcher::generator::capture_entry();
    let image = std::env::current_exe().map_err(|e| e.to_string())?;
    if image
        .parent()
        .is_some_and(|root| root.join("mcp/uninstall.v1.json").exists())
    {
        let arguments = std::env::args().skip(1).collect::<Vec<_>>();
        if arguments != ["plugin", "retire-installation"] {
            return Err("installation_uninstall_in_progress".into());
        }
    }
    let runtime = RuntimeIdentity::discover()?;
    let _ = RUNTIME.set(runtime.clone());
    let Some(runtime) = runtime else {
        if std::env::var_os("SE_MCP_MAINTENANCE").as_deref() == Some(std::ffi::OsStr::new("1")) {
            return admit_maintenance();
        }
        if std::env::var_os("SE_MCP_ADMISSION_CHANNEL").is_some() {
            return Err("runtime_admission_image_outside_generation".into());
        }
        return Ok(ProcessAdmission {
            _lifetime: None,
            _operation: None,
        });
    };
    let started_launcher = std::env::var("SE_MCP_STARTER_SHA256").ok();
    std::env::remove_var("SE_MCP_STARTER_SHA256");
    let info = started_launcher
        .map(|hash| {
            if !runtime
                .installation
                .manifest
                .allowed_launcher_hashes
                .contains(&hash)
            {
                return Err("launcher_bootstrap_hash_not_allowed".to_string());
            }
            let path = runtime
                .installation
                .root
                .join("mcp/launchers")
                .join(&hash)
                .join("info.v1.json");
            os::validate_tree(&runtime.installation.root, &path)?;
            let mut info: serde_json::Value = crate::mcp_launcher::installation::read_json(&path)?;
            info["sha256"] = serde_json::json!(hash);
            Ok(info)
        })
        .transpose()?;
    let _ = STARTER.set(info);
    let lifetime = runtime.lifetime()?;
    let is_server = std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("mcp"));
    let migration =
        std::env::var_os("SE_MCP_MIGRATION_ENTRY").as_deref() == Some(std::ffi::OsStr::new("1"));
    std::env::remove_var("SE_MCP_MIGRATION_ENTRY");
    let _ = MIGRATION_ENTRY.set(is_server && migration);
    let guard = if is_server {
        check_start(&runtime.installation)?;
        None
    } else {
        Some(acquire_with(
            &runtime,
            std::env::var_os("SE_MCP_ADMISSION_CHANNEL").is_some(),
        )?)
    };
    if let Some((mut channel, lifecycle, mut acknowledgement)) = os::take_admission_channel()? {
        os::acknowledge(acknowledgement.as_mut().unwrap_or(&mut channel))?;
        if lifecycle {
            if !is_server {
                return Err("runtime_lifecycle_only_for_mcp".into());
            }
            *LIFECYCLE.lock().map_err(|_| "runtime_lifecycle_poisoned")? = Some(channel);
        }
    } else if is_server {
        return Err("runtime_direct_start_refused_use_registered_launcher".into());
    }
    Ok(ProcessAdmission {
        _lifetime: Some(lifetime),
        _operation: guard,
    })
}
fn check_start(installation: &Installation) -> Result<(), String> {
    let _control = installation.lock("control.lock", false)?;
    check_state(installation)?;
    Ok(())
}
pub(crate) fn check_state(installation: &Installation) -> Result<(), String> {
    if installation.root.join("mcp/uninstall.v1.json").exists() {
        return Err("installation_uninstall_in_progress".into());
    }
    if let Some(state) = installation.state()? {
        if state.blocks_work()
            || (state.retire_managed
                && state.phase == crate::mcp_launcher::core::lifecycle::Phase::Committed
                && installation.manifest == state.previous)
        {
            return Err(if installation.owner_alive(&state) {
                "update_in_progress"
            } else {
                "update_recovery_required"
            }
            .into());
        }
    }
    Ok(())
}
fn acquire(runtime: &RuntimeIdentity) -> Result<OperationGuard, String> {
    acquire_with(runtime, false)
}
fn check_admission(
    installation: &Installation,
    runtime: &RuntimeIdentity,
    continuation: bool,
) -> Result<(), String> {
    if !installation.manifest.permits(&runtime.contract) {
        return Err("reconnect_required: runtime hash or contract is not admitted".into());
    }
    if continuation {
        if let Some(state) = installation.state()? {
            if state.blocks_work() {
                if !installation.owner_alive(&state) {
                    return Err("update_recovery_required".into());
                }
                if state.phase != crate::mcp_launcher::core::lifecycle::Phase::Prepared {
                    return Err("update_in_progress".into());
                }
                // Only the authenticated continuation path uses this admission.
                // Its caller still owns the prior shared lease, so Drain cannot
                // obtain exclusive access between that caller and this handle.
                return Ok(());
            }
        }
    }
    check_state(installation)
}
fn acquire_with(runtime: &RuntimeIdentity, continuation: bool) -> Result<OperationGuard, String> {
    let root = &runtime.installation.root;
    let installation = Installation::load(root)?;
    {
        let _control = installation.lock("control.lock", false)?;
        let current = Installation::load(root)?;
        check_admission(&current, runtime, continuation)?;
    }
    let lifetime = runtime.lifetime()?;
    let lock = installation.lock("operations.lock", false)?;
    // Read the confirmed pointer again while observing control: a prior
    // manifest snapshot cannot authorize fresh incompatible work after Commit.
    {
        let _control = installation.lock("control.lock", false)?;
        let current = Installation::load(root)?;
        check_admission(&current, runtime, continuation)?;
    }
    Ok(OperationGuard {
        _lock: lock,
        _lifetime: lifetime,
    })
}
pub(crate) fn continuation_operation() -> Result<Option<OperationGuard>, String> {
    current_runtime()
        .map(|runtime| acquire_with(&runtime, true))
        .transpose()
}
pub fn operation() -> Result<Option<OperationGuard>, String> {
    current_runtime()
        .map(|runtime| acquire(&runtime))
        .transpose()
}

fn admit_maintenance() -> Result<ProcessAdmission, String> {
    use std::io::Read;
    std::env::remove_var("SE_MCP_MAINTENANCE");
    let installation = Installation::current()?.ok_or("maintenance_installation_missing")?;
    let image = std::fs::canonicalize(std::env::current_exe().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if image != installation.installed_cli() {
        return Err("maintenance_image_mismatch".into());
    }
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if !crate::mcp_launcher::is_maintenance(&arguments) {
        return Err("maintenance_request_type_invalid".into());
    }
    let _verified = os::open_verified(
        &installation.root,
        &image,
        &installation.manifest.runtime_sha256,
    )?;
    let runtime = RuntimeIdentity {
        installation: installation.clone(),
        contract: crate::mcp_launcher::RuntimeContract {
            runtime_sha256: installation.manifest.runtime_sha256.clone(),
            ipc_contract_id: installation.manifest.ipc_contract_id.clone(),
            shared_state_contract_id: installation.manifest.shared_state_contract_id.clone(),
        },
        executable: installation.runtime_path(),
    };
    let lifetime = runtime.lifetime()?;
    let operation = acquire_with(&runtime, true)?;
    let (mut channel, lifecycle, mut ack) =
        os::take_admission_channel()?.ok_or("maintenance_private_channel_required")?;
    if lifecycle {
        return Err("maintenance_lifecycle_invalid".into());
    }
    os::acknowledge(ack.as_mut().unwrap_or(&mut channel))?;
    let mut release = [0u8];
    channel
        .read_exact(&mut release)
        .map_err(|e| format!("maintenance_handoff_not_released: {e}"))?;
    if release != [2] {
        return Err("maintenance_handoff_release_invalid".into());
    }
    drop(operation);
    Ok(ProcessAdmission {
        _lifetime: Some(lifetime),
        _operation: None,
    })
}

/// Server-side lifetime for an authenticated generation-owned request. The
/// same Arc may be retained by a queued owner; its final drop releases the
/// operation and generation locks, with no lock inheritance or admission gap.
pub fn operation_for_contract(
    installation_id: &str,
    contract: &crate::mcp_launcher::RuntimeContract,
) -> Result<std::sync::Arc<OperationGuard>, String> {
    let installation = Installation::current()?.ok_or("runtime_installation_missing")?;
    if installation.manifest.installation_id != installation_id {
        return Err("runtime_installation_id_mismatch".into());
    }
    let executable = installation
        .root
        .join("mcp/runtimes")
        .join(&contract.runtime_sha256)
        .join(os::CLI_NAME);
    let runtime = RuntimeIdentity {
        installation,
        contract: contract.clone(),
        executable,
    };
    acquire_with(&runtime, true).map(std::sync::Arc::new)
}

/// The captured running image, fresh manifest and held operation jointly bind
/// the templates. A replaced pathname alone cannot identify the running code.
pub fn generator_operation() -> Result<Option<OperationGuard>, String> {
    crate::mcp_launcher::generator::identity()?
        .map(|runtime| acquire(&runtime))
        .transpose()
}
