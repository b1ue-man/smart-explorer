//! An update its owner left in phase "stopping" (field failure of
//! 2026-11-17) on real file and lock boundaries, without a subprocess.
use super::*;
use crate::mcp_launcher::core::lifecycle::InstallationTarget;
use crate::mcp_launcher::core::manifest::{Manifest, IPC_CONTRACT, STATE_CONTRACT};
use crate::mcp_launcher::installation::{atomic_json, hash_bytes};
use crate::mcp_launcher::UpdateTransaction;

const RUNTIME: &[u8] = b"\x7fELF-update-sha-runtime-fixture";
const NEW_CLI: &[u8] = b"\x7fELF-update-sha-new-cli";

struct Fixture {
    _directory: tempfile::TempDir,
    installation: Installation,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        os::protect(&root, false).unwrap();
        // Deliberately non-runnable bytes: no step of these tests starts them.
        let launcher = b"\x7fELF-update-sha-launcher-fixture";
        let (runtime_hash, launcher_hash) = (hash_bytes(RUNTIME), hash_bytes(launcher));
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
            "mcp".to_owned(),
            "mcp/runtimes".into(),
            format!("mcp/runtimes/{runtime_hash}"),
            "mcp/launchers".into(),
            format!("mcp/launchers/{launcher_hash}"),
        ] {
            std::fs::create_dir(root.join(&relative)).unwrap();
            os::protect(&root.join(relative), false).unwrap();
        }
        let write = |relative: &str, bytes: &[u8], executable| {
            std::fs::write(root.join(relative), bytes).unwrap();
            os::protect(&root.join(relative), executable).unwrap();
        };
        write(&manifest.runtime_file, RUNTIME, true);
        write(os::CLI_NAME, RUNTIME, true);
        let launcher_file = format!("mcp/launchers/{launcher_hash}/{}", os::LAUNCHER_NAME);
        write(&launcher_file, launcher, true);
        write("mcp/installation.id", b"fixture-installation", false);
        for name in ["update-owner.lock", "control.lock", "operations.lock"] {
            write(&format!("mcp/{name}"), b"", false);
        }
        write(
            &format!("mcp/runtimes/{runtime_hash}/lifetime.lock"),
            b"",
            false,
        );
        atomic_json(&root.join("mcp/current.v1.json"), &manifest).unwrap();
        let installation = Installation::load(&root).unwrap();
        Self {
            _directory: directory,
            installation,
        }
    }

    /// Prepare, drain and stop, as the updater does before it replaces files.
    fn stopping(&self, version: &str) -> UpdateTransaction {
        let proof = InstallationTarget {
            file: os::CLI_NAME.into(),
            previous_sha256: Some(hash_bytes(RUNTIME)),
            target_sha256: hash_bytes(NEW_CLI),
        };
        let root = &self.installation.root;
        let mut transaction =
            UpdateTransaction::prepare_retirement(root, version, vec![proof]).unwrap();
        transaction.drain(Duration::ZERO).unwrap();
        transaction.publish_stop(&root.join("daemon.stop")).unwrap();
        transaction
    }

    /// The owner ended without rollback: its lock is free and its recorded
    /// process identity names no running process any more.
    fn abandoned(&self) {
        drop(self.stopping("1.0.1"));
        let mut state = self.installation.state().unwrap().unwrap();
        assert_eq!(state.phase, Phase::Stopping);
        state.owner.started.push_str("-ended");
        atomic_json(&self.installation.root.join("mcp/update.v1.json"), &state).unwrap();
    }

    fn phase(&self) -> Phase {
        self.installation.state().unwrap().unwrap().phase
    }
}

#[test]
fn update_sha_task_stopping_leftover_with_previous_files_rolls_back_and_admits_a_new_update() {
    let fixture = Fixture::new();
    let installation = &fixture.installation;
    let root = &installation.root;
    fixture.abandoned();
    let blocked = crate::mcp_launcher::admission::check_state(installation).unwrap_err();
    assert_eq!(blocked, "update_recovery_required");
    // The check the updater ran instead before: completion against previous files.
    let masked = crate::mcp_launcher::verify_recovery_targets(root, true).unwrap_err();
    assert_eq!(masked, "runtime_sha256_mismatch");
    let found = inspect_interrupted_update(root).unwrap().unwrap();
    // What the app's update page and `se update --check` show.
    let pending = crate::updater::PendingUpdate::RollBack {
        previous_version: "1.0.0".into(),
        target_version: "1.0.1".into(),
    };
    assert_eq!(crate::updater::pending_update_at(root), Some(pending));
    assert_eq!(
        (found.resolution, found.running),
        (Resolution::RollBack, false)
    );

    assert_eq!(
        resume_interrupted_update(root, Duration::ZERO).unwrap(),
        Resumed::RolledBack
    );
    assert_eq!(fixture.phase(), Phase::RolledBack);
    crate::mcp_launcher::admission::check_state(installation).unwrap();
    assert_eq!(
        Installation::load(root).unwrap().manifest,
        installation.manifest
    );
    // Idempotent: a second attempt finds nothing, and the next update prepares.
    assert_eq!(
        resume_interrupted_update(root, Duration::ZERO).unwrap(),
        Resumed::Nothing
    );
    assert!(inspect_interrupted_update(root).unwrap().is_none());
    assert_eq!(crate::updater::pending_update_at(root), None);
    fixture.stopping("1.0.2").rollback().unwrap();
}

#[test]
fn update_sha_task_stopping_leftover_with_target_files_waits_for_the_start_proof() {
    let fixture = Fixture::new();
    let root = &fixture.installation.root;
    fixture.abandoned();
    std::fs::write(fixture.installation.installed_cli(), NEW_CLI).unwrap();
    assert_eq!(
        resume_interrupted_update(root, Duration::ZERO).unwrap(),
        Resumed::CompletionPending
    );
    assert_eq!(
        fixture.phase(),
        Phase::Stopping,
        "completion needs the caller's start proof"
    );
}

#[test]
fn update_sha_task_stopping_leftover_with_unknown_files_is_refused_by_name() {
    let fixture = Fixture::new();
    let root = &fixture.installation.root;
    fixture.abandoned();
    std::fs::write(fixture.installation.installed_cli(), b"\x7fELF-foreign").unwrap();
    let error = resume_interrupted_update(root, Duration::ZERO).unwrap_err();
    assert!(error.starts_with("update_recovery_refused: "), "{error}");
    assert!(matches!(
        crate::updater::pending_update_at(root),
        Some(crate::updater::PendingUpdate::Refused { .. })
    ));
    assert!(
        error.contains(&format!("{} hat unbekannten Inhalt", os::CLI_NAME)),
        "{error}"
    );
    assert_eq!(fixture.phase(), Phase::Stopping);
}

#[test]
fn update_sha_task_rollback_waits_for_a_reader_holding_control_briefly() {
    let fixture = Fixture::new();
    let root = fixture.installation.root.clone();
    fixture.abandoned();
    let reader = fixture.installation.clone();
    let (held, wait_held) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let control = reader.lock("control.lock", false).unwrap();
        held.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(400));
        drop(control);
    });
    wait_held.recv().unwrap();
    // A fail-fast request loses against the reader, as the update owner's did.
    let busy = fixture.installation.lock("control.lock", true).unwrap_err();
    assert!(busy.starts_with("lock_busy:"), "{busy}");
    assert_eq!(
        resume_interrupted_update(&root, Duration::ZERO).unwrap(),
        Resumed::RolledBack
    );
    thread.join().unwrap();
    assert_eq!(fixture.phase(), Phase::RolledBack);
}

#[test]
fn update_sha_task_running_or_exiting_owner_is_not_recovered() {
    let fixture = Fixture::new();
    let root = &fixture.installation.root;
    let transaction = fixture.stopping("1.0.1");
    assert_eq!(
        resume_interrupted_update(root, Duration::ZERO).unwrap(),
        Resumed::Running
    );
    assert!(inspect_interrupted_update(root).unwrap().unwrap().running);
    // The owner lock is free, but the recorded process (this one) still runs.
    drop(transaction);
    let error = resume_interrupted_update(root, Duration::from_millis(300)).unwrap_err();
    assert!(error.starts_with("update_owner_still_alive: "), "{error}");
    assert_eq!(fixture.phase(), Phase::Stopping);
}
