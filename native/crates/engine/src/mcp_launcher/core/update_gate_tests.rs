//! No worker starts from the image an update is replacing (field failure
//! 2026-11-18, 0.5.199 -> 0.5.200 on Windows with an MCP client attached).
use super::*;
use crate::mcp_launcher::core::lifecycle::{InstallationTarget, ProcessIdentity};
use crate::mcp_launcher::core::manifest::{IPC_CONTRACT, STATE_CONTRACT};

fn hash(prefix: &str) -> String {
    format!("{prefix}{}", "0".repeat(64 - prefix.len()))
}

fn manifest(generation: u64, runtime: &str) -> Manifest {
    Manifest {
        schema: 1,
        installation_id: "fixture-installation".into(),
        committed_transaction_id: format!("fixture-{generation}"),
        product_version: format!("0.5.{generation}"),
        generation,
        runtime_sha256: hash(runtime),
        runtime_file: format!("mcp/runtimes/{}/se.exe", hash(runtime)),
        launcher_protocol: 1,
        launcher_sha256: hash("c7"),
        launcher_product_version: "0.5.200".into(),
        allowed_launcher_hashes: vec![hash("c7")],
        ipc_contract_id: IPC_CONTRACT.into(),
        shared_state_contract_id: STATE_CONTRACT.into(),
        supported_runtimes: vec![],
        compatibility_catalog_sha256: "d".repeat(64),
    }
}

fn state(phase: Phase, retire_managed: bool) -> UpdateState {
    UpdateState {
        schema: 1,
        installation_id: "fixture-installation".into(),
        transaction_id: "fixture-200".into(),
        target_manifest_sha256: "e".repeat(64),
        owner: ProcessIdentity {
            pid: 4711,
            started: "1:2".into(),
            image: "Smart Explorer Updater.exe".into(),
            executable_object: None,
        },
        phase,
        target: manifest(200, "2c"),
        previous: manifest(199, "4a"),
        stop: None,
        legacy_stop: None,
        installation_targets: vec![InstallationTarget {
            file: "se.exe".into(),
            previous_sha256: Some(hash("4a")),
            target_sha256: hash("2c"),
        }],
        retire_managed,
    }
}

#[test]
fn update_mcp_task_no_worker_starts_while_an_update_holds_the_installation() {
    let previous = manifest(199, "4a");
    let target = manifest(200, "2c");
    let cases = [
        ("no record", None, &previous, true, StartGate::Open),
        (
            "prepared, owner alive",
            Some(state(Phase::Prepared, false)),
            &previous,
            true,
            StartGate::UpdateRunning,
        ),
        (
            "stopping, owner alive",
            Some(state(Phase::Stopping, false)),
            &previous,
            true,
            StartGate::UpdateRunning,
        ),
        (
            "stopping, owner gone",
            Some(state(Phase::Stopping, false)),
            &previous,
            false,
            StartGate::RecoveryRequired,
        ),
        (
            "committed",
            Some(state(Phase::Committed, false)),
            &target,
            true,
            StartGate::Open,
        ),
        (
            "rolled back",
            Some(state(Phase::RolledBack, false)),
            &previous,
            true,
            StartGate::Open,
        ),
        (
            "committed retirement, pointer still previous",
            Some(state(Phase::Committed, true)),
            &previous,
            true,
            StartGate::UpdateRunning,
        ),
    ];
    for (case, record, current, alive, expected) in cases {
        let blocked = blocks(record.as_ref(), current);
        assert_eq!(gate(blocked, || alive), expected, "{case}");
    }
    // The owner is asked only for a blocking record.
    assert_eq!(
        gate(false, || panic!("owner asked without a record")),
        StartGate::Open
    );
}

#[test]
fn update_mcp_task_a_worker_start_waits_for_the_update_then_names_it() {
    assert_eq!(worker_start(StartGate::Open, false), WorkerStart::Start);
    assert_eq!(worker_start(StartGate::Open, true), WorkerStart::Start);
    assert_eq!(
        worker_start(StartGate::UpdateRunning, false),
        WorkerStart::WaitAgain
    );
    assert_eq!(
        worker_start(StartGate::UpdateRunning, true),
        WorkerStart::Refuse(UPDATE_RUNNING)
    );
    assert_eq!(
        worker_start(StartGate::RecoveryRequired, false),
        WorkerStart::Refuse(UPDATE_RECOVERY)
    );
    // Relay, daemon IPC and updater match these prefixes.
    assert!(UPDATE_RUNNING.starts_with("update_in_progress"));
    assert!(UPDATE_RUNNING.contains("Smart Explorer wird gerade aktualisiert"));
    assert!(UPDATE_RECOVERY.starts_with("update_recovery_required"));
}

#[test]
fn update_mcp_task_an_ordinary_start_keeps_the_stop_of_a_running_update() {
    assert!(!ordinary_start_allowed(true, || true));
    assert!(ordinary_start_allowed(true, || false));
    // A handoff or an absent request never asks the installation.
    assert!(ordinary_start_allowed(false, || panic!(
        "asked without a stop"
    )));
}
