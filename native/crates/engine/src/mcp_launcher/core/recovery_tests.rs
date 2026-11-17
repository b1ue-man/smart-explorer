//! The direction of an update its owner left unfinished (field failure of
//! 2026-11-17: a leftover "stopping" record on a Windows installation whose
//! files were still the previous version).
use super::lifecycle::{InstallationTarget, Phase, ProcessIdentity, UpdateState};
use super::manifest::{Manifest, IPC_CONTRACT, STATE_CONTRACT};
use super::recovery::{found, refusal, resolve, FoundTarget, Resolution};

const PREVIOUS_CLI: &str = "4adf5460";
const TARGET_CLI: &str = "2c910c05";

fn hash(prefix: &str) -> String {
    format!("{prefix}{}", "0".repeat(64 - prefix.len()))
}

fn manifest(version: &str, generation: u64, runtime: &str) -> Manifest {
    Manifest {
        schema: 1,
        installation_id: "fixture-installation".into(),
        committed_transaction_id: format!("fixture-{generation}"),
        product_version: version.into(),
        generation,
        runtime_sha256: hash(runtime),
        runtime_file: format!("mcp/runtimes/{}/se.exe", hash(runtime)),
        launcher_protocol: 1,
        launcher_sha256: hash("c7193c67"),
        launcher_product_version: version.into(),
        allowed_launcher_hashes: vec![hash("c7193c67")],
        ipc_contract_id: IPC_CONTRACT.into(),
        shared_state_contract_id: STATE_CONTRACT.into(),
        supported_runtimes: vec![],
        compatibility_catalog_sha256: "d".repeat(64),
    }
}

/// The record of the field failure: three regular files, phase "stopping".
fn state() -> UpdateState {
    let target = |file: &str, previous: &str, target: &str| InstallationTarget {
        file: file.into(),
        previous_sha256: Some(hash(previous)),
        target_sha256: hash(target),
    };
    UpdateState {
        schema: 1,
        installation_id: "fixture-installation".into(),
        transaction_id: "fixture-13".into(),
        target_manifest_sha256: "e".repeat(64),
        owner: ProcessIdentity {
            pid: 11984,
            started: "1:2".into(),
            image: "updater_download_0.5.198.exe".into(),
            executable_object: None,
        },
        phase: Phase::Stopping,
        target: manifest("0.5.198", 13, TARGET_CLI),
        previous: manifest("0.5.197", 12, PREVIOUS_CLI),
        stop: None,
        legacy_stop: None,
        installation_targets: vec![
            target("se.exe", PREVIOUS_CLI, TARGET_CLI),
            target("Smart Explorer Updater.exe", "88a94172", "1e5ac5a8"),
            target("Smart Explorer.exe", "19ecdfbc", "9bec6002"),
        ],
        retire_managed: false,
    }
}

fn found_with(state: &UpdateState, hashes: [Option<&str>; 3]) -> Vec<FoundTarget> {
    state
        .installation_targets
        .iter()
        .zip(hashes)
        .map(|(target, prefix)| found(target, prefix.map(hash)))
        .collect()
}

#[test]
fn update_sha_task_previous_files_roll_back() {
    let state = state();
    let files = found_with(
        &state,
        [Some(PREVIOUS_CLI), Some("88a94172"), Some("19ecdfbc")],
    );
    assert_eq!(resolve(&state, &files), Resolution::RollBack);
}

#[test]
fn update_sha_task_target_files_complete() {
    let state = state();
    let files = found_with(
        &state,
        [Some(TARGET_CLI), Some("1e5ac5a8"), Some("9bec6002")],
    );
    assert_eq!(resolve(&state, &files), Resolution::Complete);
}

#[test]
fn update_sha_task_mixed_or_unknown_files_are_refused_and_named() {
    let state = state();
    let mixed = found_with(
        &state,
        [Some(PREVIOUS_CLI), Some("1e5ac5a8"), Some("19ecdfbc")],
    );
    let Resolution::Refuse(entries) = resolve(&state, &mixed) else {
        panic!("a half-replaced installation is neither direction");
    };
    let text = refusal(&state, &entries);
    assert!(text.starts_with("update_recovery_refused: "), "{text}");
    assert!(text.contains("se.exe ist v0.5.197"), "{text}");
    assert!(
        text.contains("Smart Explorer Updater.exe ist v0.5.198"),
        "{text}"
    );
    assert!(text.contains("Smart Explorer.exe ist v0.5.197"), "{text}");

    let unknown = found_with(&state, [Some("ffff0000"), Some("88a94172"), None]);
    let Resolution::Refuse(entries) = resolve(&state, &unknown) else {
        panic!("unknown content is never rolled back");
    };
    let text = refusal(&state, &entries);
    assert!(
        text.contains("se.exe hat unbekannten Inhalt (SHA-256 ffff00000000)"),
        "{text}"
    );
    assert!(text.contains("Smart Explorer.exe fehlt"), "{text}");
}

#[test]
fn update_sha_task_absent_files_and_unchanged_payloads_follow_the_record() {
    let mut state = state();
    // A file absent before the update is "previous" only while it is absent.
    state.installation_targets[2].previous_sha256 = None;
    let files = found_with(&state, [Some(PREVIOUS_CLI), Some("88a94172"), None]);
    assert_eq!(resolve(&state, &files), Resolution::RollBack);
    let files = found_with(
        &state,
        [Some(PREVIOUS_CLI), Some("88a94172"), Some("9bec6002")],
    );
    assert!(matches!(resolve(&state, &files), Resolution::Refuse(_)));
    // A payload equal in both versions counts for either direction.
    state.installation_targets[1].target_sha256 = hash("88a94172");
    let files = found_with(
        &state,
        [Some(TARGET_CLI), Some("88a94172"), Some("9bec6002")],
    );
    assert_eq!(resolve(&state, &files), Resolution::Complete);
    // A recorded file the caller did not report, and a record without files.
    assert!(matches!(
        resolve(&state, &files[..2]),
        Resolution::Refuse(_)
    ));
    state.installation_targets.clear();
    assert!(matches!(resolve(&state, &[]), Resolution::Refuse(_)));
}
