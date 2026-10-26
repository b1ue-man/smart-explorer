//! Verified immutable MCP generations, installation-wide admission and update lifecycle.
pub mod core;
#[path = "os/shared/generator.rs"]
mod generator;
pub use generator::{refresh_installed_profile,refresh_entry_installation};
pub(crate) use generator::{legacy_entry_path,legacy_profile_guard};
#[path = "os/shared/contract_probe.rs"]
mod contract_probe;
pub use contract_probe::{runtime_contract_probe,verify_legacy_product};
#[path = "os/shared/compatibility.rs"]
mod compatibility;
pub use compatibility::{compatible_runtimes, compatible_launchers, CompatibilitySelection, CompatibilityRejection};
#[path = "os/shared/installer_cli.rs"]
mod installer_cli;
pub(crate) use installer_cli::retire_current_installation;
#[path = "os/mod.rs"]
mod os;
#[path = "os/shared/installation.rs"]
mod installation;
#[path = "os/shared/admission.rs"]
mod admission;
#[path = "os/shared/transaction.rs"]
mod transaction;
#[path = "os/shared/provision.rs"]
mod provision;
#[path = "os/shared/launcher.rs"]
mod launcher;

#[path = "os/shared/child.rs"]
mod child;
pub use child::GenerationChild;

pub(crate) use admission::continuation_operation;
pub use admission::{admit_cli_process, current_runtime, current_starter, migration_entry, operation, operation_for_contract, generator_operation, ProcessAdmission, OperationGuard};
pub use installation::{Installation, RuntimeIdentity};
pub use launcher::{launch, run_launcher, spawn_generation_child, launch_from_installed, is_maintenance, refresh_current_profile, start_current_daemon, LifecycleWatch};
pub use provision::{install_generation, stage_generation, staged_runtime_managed, verify_staged_launcher, GenerationInput, InitialGenerationLease, commit_initial_generation, new_installation_id, disable_legacy_generation, validate_installation_root, validate_installation_path, verify_installed_product};
pub use transaction::{UpdateTransaction, recover_update, verify_recovery_targets, garbage_collect, stop_marker_is_owned, retain_update_stop, retained_update_transaction, stop_marker_identity, installation_stop_marker};
pub use core::manifest::{Manifest, RuntimeContract, IPC_CONTRACT, STATE_CONTRACT};
pub use core::lifecycle::{RuntimeHello, RuntimeHelloReply};

pub(crate) fn legacy_profile_image(path:&std::path::Path)->Result<std::fs::File,String> {os::legacy_profile_image(path)}
pub(crate) fn legacy_update_image(path:&std::path::Path)->Result<Option<std::fs::File>,String> {os::legacy_update_image(path)}
pub(crate) fn same_image_file(file:&std::fs::File,path:&std::path::Path)->Result<bool,String> {os::same_image_file(file,path)}

pub fn status_value() -> serde_json::Value {
    match Installation::current() {
        Ok(Some(installation)) => installation.status(),
        Ok(None) => serde_json::json!({"state":"legacy", "reconnect_required":true}),
        Err(error) => serde_json::json!({"state":"invalid", "error":error, "reconnect_required":true}),
    }
}
