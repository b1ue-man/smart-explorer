//! Verified immutable MCP generations, installation-wide admission and update lifecycle.
pub mod core;
#[path = "os/shared/generator.rs"]
mod generator;
pub(crate) use generator::{legacy_entry_path, legacy_profile_guard};
pub use generator::{refresh_entry_installation, refresh_installed_profile};
#[path = "os/shared/contract_probe.rs"]
mod contract_probe;
pub use contract_probe::{runtime_contract_probe, verify_legacy_product};
#[path = "os/shared/compatibility.rs"]
mod compatibility;
pub use compatibility::{
    compatible_launchers, compatible_runtimes, CompatibilityRejection, CompatibilitySelection,
};
#[path = "os/shared/installer_cli.rs"]
mod installer_cli;
pub(crate) use installer_cli::retire_current_installation;
#[path = "os/shared/admission.rs"]
mod admission;
#[path = "os/shared/installation.rs"]
mod installation;
#[path = "os/shared/launcher.rs"]
mod launcher;
#[path = "os/mod.rs"]
mod os;
#[path = "os/shared/provision.rs"]
mod provision;
#[path = "os/shared/transaction.rs"]
mod transaction;

#[path = "os/shared/child.rs"]
mod child;
pub use child::GenerationChild;

pub(crate) use admission::continuation_operation;
pub use admission::{
    admit_cli_process, current_runtime, current_starter, generator_operation, migration_entry,
    operation, operation_for_contract, OperationGuard, ProcessAdmission,
};
pub use core::lifecycle::{RuntimeHello, RuntimeHelloReply};
pub use core::manifest::{Manifest, RuntimeContract, IPC_CONTRACT, STATE_CONTRACT};
pub use installation::{Installation, RuntimeIdentity};
pub use launcher::{
    is_maintenance, launch, launch_from_installed, refresh_current_profile, run_launcher,
    spawn_generation_child, start_current_daemon, LifecycleWatch,
};
pub use provision::{
    commit_initial_generation, disable_legacy_generation, install_generation, new_installation_id,
    stage_generation, staged_runtime_managed, validate_installation_path,
    validate_installation_root, verify_installed_product, verify_staged_launcher, GenerationInput,
    InitialGenerationLease,
};
pub use transaction::{
    garbage_collect, installation_stop_marker, recover_update, retain_update_stop,
    retained_update_transaction, stop_marker_identity, stop_marker_is_owned,
    verify_recovery_targets, UpdateTransaction,
};

pub(crate) fn legacy_profile_image(path: &std::path::Path) -> Result<std::fs::File, String> {
    os::legacy_profile_image(path)
}
pub(crate) fn legacy_update_image(path: &std::path::Path) -> Result<Option<std::fs::File>, String> {
    os::legacy_update_image(path)
}
pub(crate) fn same_image_file(
    file: &std::fs::File,
    path: &std::path::Path,
) -> Result<bool, String> {
    os::same_image_file(file, path)
}

pub fn status_value() -> serde_json::Value {
    match Installation::current() {
        Ok(Some(installation)) => installation.status(),
        Ok(None) => serde_json::json!({"state":"legacy", "reconnect_required":true}),
        Err(error) => {
            serde_json::json!({"state":"invalid", "error":error, "reconnect_required":true})
        }
    }
}
