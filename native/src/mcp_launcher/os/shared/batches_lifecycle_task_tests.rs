//! Real file/lock boundaries, without launching a runtime or touching user profiles.
use super::*;
use crate::mcp_launcher::core::manifest::{IPC_CONTRACT, STATE_CONTRACT};
use crate::mcp_launcher::RuntimeIdentity;

struct Fixture {
    _directory: tempfile::TempDir,
    installation: Installation,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        os::protect(&root, false).unwrap();
        // These are deliberately non-runnable image bytes. No fixture invokes spawn.
        let runtime = b"\x7fELF-p13-runtime-fixture";
        let launcher = b"\x7fELF-p13-launcher-fixture";
        let runtime_hash = hash_bytes(runtime);
        let launcher_hash = hash_bytes(launcher);
        let manifest = Manifest {
            schema: 1,
            installation_id: "fixture-installation".into(),
            committed_transaction_id: "fixture-original".into(),
            product_version: "1.0.0".into(),
            generation: 1,
            runtime_sha256: runtime_hash.clone(),
            runtime_file: format!("mcp/runtimes/{runtime_hash}/{}", os::CLI_NAME),
            launcher_protocol: 1,
            launcher_sha256: launcher_hash.clone(),
            launcher_product_version: "1.0.0".into(),
            allowed_launcher_hashes: vec![launcher_hash.clone()],
            ipc_contract_id: IPC_CONTRACT.into(),
            shared_state_contract_id: STATE_CONTRACT.into(),
            supported_runtimes: vec![],
            compatibility_catalog_sha256: "a".repeat(64),
        };
        for relative in [
            "mcp".to_owned(), "mcp/runtimes".into(),
            format!("mcp/runtimes/{runtime_hash}"), "mcp/launchers".into(),
            format!("mcp/launchers/{launcher_hash}"),
        ] {
            let path = root.join(relative);
            std::fs::create_dir(&path).unwrap();
            os::protect(&path, false).unwrap();
        }
        let write = |relative: &str, bytes: &[u8], executable| {
            let path = root.join(relative);
            std::fs::write(&path, bytes).unwrap();
            os::protect(&path, executable).unwrap();
        };
        write(&manifest.runtime_file, runtime, true);
        write(os::CLI_NAME, runtime, true);
        write(&format!("mcp/launchers/{launcher_hash}/{}", os::LAUNCHER_NAME), launcher, true);
        write("mcp/installation.id", manifest.installation_id.as_bytes(), false);
        for name in ["update-owner.lock", "control.lock", "operations.lock"] {
            write(&format!("mcp/{name}"), b"", false);
        }
        write(&format!("mcp/runtimes/{runtime_hash}/lifetime.lock"), b"", false);
        atomic_json(&root.join("mcp/current.v1.json"), &manifest).unwrap();
        let installation = Installation::load(&root).unwrap();
        Self { _directory: directory, installation }
    }

    fn prepare(&self) -> UpdateTransaction {
        let hash = &self.installation.manifest.runtime_sha256;
        // Retirement exercises the real prepare/owner/state path without a subprocess probe.
        UpdateTransaction::prepare_retirement(&self.installation.root, "1.0.1", vec![
            InstallationTarget {
                file: os::CLI_NAME.into(),
                previous_sha256: Some(hash.clone()),
                target_sha256: hash.clone(),
            },
        ]).unwrap()
    }
}

#[test]
fn batches_p13_owner_requires_lock_and_exact_process_identity() {
    let fixture = Fixture::new();
    let transaction = fixture.prepare();
    let installation = &fixture.installation;
    assert!(installation.owner_alive(&transaction.state));
    assert_eq!(crate::mcp_launcher::admission::check_state(installation).unwrap_err(), "update_in_progress");
    assert!(installation.lock("update-owner.lock", true).unwrap_err().starts_with("lock_busy:"));
    let mut forged = transaction.state.clone();
    forged.owner.started.push_str("-reused-pid");
    assert!(!installation.owner_alive(&forged));
    drop(transaction);
    let abandoned = installation.state().unwrap().unwrap();
    assert!(!installation.owner_alive(&abandoned), "live PID without owner lock is insufficient");
    assert_eq!(crate::mcp_launcher::admission::check_state(installation).unwrap_err(), "update_recovery_required");
    assert_eq!(Installation::load(&installation.root).unwrap().manifest, installation.manifest);
}

#[test]
fn batches_p13_busy_drain_rolls_back_without_stopping_active_work() {
    let fixture = Fixture::new();
    let installation = &fixture.installation;
    let active = installation.lock("operations.lock", false).unwrap();
    let mut transaction = fixture.prepare();
    // Prepare has released control before drain; status/cancellation remain possible.
    drop(installation.lock("control.lock", true).unwrap());
    assert!(transaction.drain(Duration::ZERO).unwrap_err().starts_with("update_deferred_busy:"));
    assert_eq!(installation.state().unwrap().unwrap().phase, Phase::RolledBack);
    crate::mcp_launcher::admission::check_state(installation).unwrap();
    assert_eq!(Installation::load(&installation.root).unwrap().manifest, installation.manifest);
    assert!(!installation.root.join("mcp/update.stop").exists());
    assert!(installation.lock("operations.lock", true).is_err());
    drop(active);
    drop(installation.lock("operations.lock", true).unwrap());
}

#[test]
fn batches_p13_stop_is_drain_gated_and_bound_to_exact_marker() {
    let fixture = Fixture::new();
    let installation = &fixture.installation;
    let marker = installation.root.join("daemon.stop");
    let mut transaction = fixture.prepare();
    assert_eq!(transaction.publish_stop(&marker).unwrap_err(), "update_stop_requires_drain");
    assert!(!marker.exists());
    transaction.drain(Duration::ZERO).unwrap();
    transaction.publish_stop(&marker).unwrap();
    assert_eq!(std::fs::read(&marker).unwrap(), b"stop");
    assert!(stop_marker_is_owned(&installation.root, &marker).unwrap());
    assert_eq!(transaction.state.legacy_stop.as_ref(), Some(&os::stop_identity(&marker).unwrap()));
    atomic_bytes(&marker, b"new independent stop request").unwrap();
    assert!(!stop_marker_is_owned(&installation.root, &marker).unwrap());
    transaction.rollback().unwrap();
}

#[test]
fn batches_p13_rollback_rejects_changed_regular_target_and_keeps_recovery_state() {
    let fixture = Fixture::new();
    let installation = &fixture.installation;
    let mut transaction = fixture.prepare();
    std::fs::write(installation.installed_cli(), b"\x7fELF-changed").unwrap();
    assert_eq!(transaction.rollback().unwrap_err(), "runtime_sha256_mismatch");
    assert_eq!(installation.state().unwrap().unwrap().phase, Phase::Prepared);
    assert_eq!(Installation::load(&installation.root).unwrap().manifest, installation.manifest);
    assert_eq!(std::fs::read(installation.installed_cli()).unwrap(), b"\x7fELF-changed");
}

#[test]
fn batches_p13_state_hash_and_installation_identity_cannot_be_substituted() {
    let fixture = Fixture::new();
    let transaction = fixture.prepare();
    let mut state = transaction.state.clone();
    state.target.product_version = "1.0.2".into();
    assert_eq!(state.validate(os::CLI_NAME).unwrap_err(), "update_state_invalid");
    state = transaction.state.clone();
    state.installation_id = "different-installation".into();
    assert_eq!(state.validate(os::CLI_NAME).unwrap_err(), "update_state_invalid");
    state = transaction.state.clone();
    state.installation_targets[0].file = "../outside".into();
    assert_eq!(state.validate(os::CLI_NAME).unwrap_err(), "update_target_proof_invalid");
    std::fs::write(fixture.installation.root.join("mcp/installation.id"), b"different-installation").unwrap();
    assert_eq!(Installation::load(&fixture.installation.root).unwrap_err(), "runtime_installation_id_mismatch");
}

#[test]
fn batches_p13_independent_lifetime_leases_survive_first_owner_drop() {
    let fixture = Fixture::new();
    let installation = fixture.installation.clone();
    let runtime = RuntimeIdentity {
        executable: installation.runtime_path(),
        contract: crate::mcp_launcher::RuntimeContract {
            runtime_sha256: installation.manifest.runtime_sha256.clone(),
            ipc_contract_id: IPC_CONTRACT.into(),
            shared_state_contract_id: STATE_CONTRACT.into(),
        },
        installation,
    };
    let path = runtime.executable.parent().unwrap().join("lifetime.lock");
    let parent = runtime.lifetime().unwrap();
    let child = runtime.lifetime().unwrap();
    assert!(os::lock(&path, true).unwrap_err().starts_with("lock_busy:"));
    drop(parent);
    assert!(os::lock(&path, true).unwrap_err().starts_with("lock_busy:"));
    drop(child);
    drop(os::lock(&path, true).unwrap());
}

#[test]
fn batches_p13_launcher_path_and_bytes_remain_bound_to_registered_installation() {
    let fixture = Fixture::new();
    let installation = &fixture.installation;
    let path = installation.checked_launcher_path().unwrap();
    assert!(installation.registered_launcher_allowed(&path).unwrap());
    assert!(!installation.registered_launcher_allowed(&installation.installed_cli()).unwrap());
    let foreign = Fixture::new();
    assert!(!installation.registered_launcher_allowed(&foreign.installation.launcher_path()).unwrap());
    std::fs::write(&path, b"\x7fELF-tampered-launcher").unwrap();
    assert_eq!(installation.checked_launcher_path().unwrap_err(), "runtime_sha256_mismatch");
    assert_eq!(installation.registered_launcher_allowed(&path).unwrap_err(), "runtime_sha256_mismatch");
}

#[test]
fn batches_p13_retention_requires_stopping_same_installation_and_supported_contract() {
    let fixture = Fixture::new();
    let mut transaction = fixture.prepare();
    let runtime = crate::mcp_launcher::RuntimeContract {
        runtime_sha256: transaction.state.previous.runtime_sha256.clone(),
        ipc_contract_id: IPC_CONTRACT.into(),
        shared_state_contract_id: STATE_CONTRACT.into(),
    };
    let id = transaction.state.installation_id.clone();
    transaction.drain(Duration::ZERO).unwrap();
    transaction.publish_stop(&fixture.installation.root.join("daemon.stop")).unwrap();
    assert!(!transaction.state.retains(&id, &runtime), "retirement never retains managed servers");
    // Pure record decision for a managed update, separate from retirement orchestration.
    let mut state = transaction.state.clone();
    state.retire_managed = false;
    assert!(state.retains(&id, &runtime));
    assert!(!state.retains("foreign-installation", &runtime));
    let mut incompatible = runtime.clone();
    incompatible.shared_state_contract_id = "unknown-state-contract".into();
    assert!(!state.retains(&id, &incompatible));
    for phase in [Phase::Prepared, Phase::Committed, Phase::RolledBack] {
        state.phase = phase;
        assert!(!state.retains(&id, &runtime));
    }
    state.phase = Phase::Stopping;
    state.stop = None;
    assert!(!state.retains(&id, &runtime));
    transaction.rollback().unwrap();
}

#[test]
fn batches_p13_historical_terminal_entry_works_offline_and_rejects_stale_image() {
    let root=tempfile::tempdir().unwrap();let cli=root.path().join("se");let bytes=b"local-cli-only";
    std::fs::write(&cli,bytes).unwrap();
    let hash=hash_bytes(bytes);
    // No manifest, launcher, feed, HTTP call, host process or profile exists.
    assert_eq!(crate::mcp_launcher::generator::verified_legacy_entry(&cli,&hash).unwrap(),cli);
    assert!(!root.path().join("mcp").exists());
    std::fs::write(&cli,b"replacement version").unwrap();
    assert_eq!(crate::mcp_launcher::generator::verified_legacy_entry(&cli,&hash).unwrap_err(),"runtime_sha256_mismatch");
    assert_eq!(std::fs::read(cli).unwrap(),b"replacement version");
}

#[test]
fn batches_p13_historical_terminal_entry_does_not_downgrade_existing_managed_state() {
    for record in ["installation.id","legacy.v1.json","update.v1.json","current.v1.json"] {
        let root=tempfile::tempdir().unwrap();let cli=root.path().join("se");let bytes=b"local-cli";
        std::fs::write(&cli,bytes).unwrap();std::fs::create_dir(root.path().join("mcp")).unwrap();
        std::fs::write(root.path().join("mcp").join(record),b"existing managed record").unwrap();
        assert!(crate::mcp_launcher::generator::verified_legacy_entry(&cli,&hash_bytes(bytes)).unwrap_err().starts_with("integration_legacy_entry_has_managed_state:"));
        assert_eq!(std::fs::read(root.path().join("mcp").join(record)).unwrap(),b"existing managed record");
    }
}
