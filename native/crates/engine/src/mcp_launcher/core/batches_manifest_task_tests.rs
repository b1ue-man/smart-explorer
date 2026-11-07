//! P13 fixtures for the shared task suite; synthetic hashes are not release approvals.
use super::capability::RuntimeProbe;
use super::manifest::{Manifest, RuntimeContract, IPC_CONTRACT, STATE_CONTRACT};

fn contract(hash: &str) -> RuntimeContract {
    RuntimeContract {
        runtime_sha256: hash.into(),
        ipc_contract_id: IPC_CONTRACT.into(),
        shared_state_contract_id: STATE_CONTRACT.into(),
    }
}

fn manifest() -> Manifest {
    Manifest {
        schema: 1,
        installation_id: "fixture-installation".into(),
        committed_transaction_id: "fixture-commit".into(),
        product_version: "1.0.0".into(),
        generation: 1,
        runtime_sha256: "a".repeat(64),
        runtime_file: format!("mcp/runtimes/{}/se", "a".repeat(64)),
        launcher_protocol: 1,
        launcher_sha256: "b".repeat(64),
        launcher_product_version: "1.0.0".into(),
        allowed_launcher_hashes: vec!["b".repeat(64)],
        ipc_contract_id: IPC_CONTRACT.into(),
        shared_state_contract_id: STATE_CONTRACT.into(),
        supported_runtimes: vec![contract(&"c".repeat(64))],
        compatibility_catalog_sha256: "d".repeat(64),
    }
}

#[test]
fn batches_p13_manifest_requires_exact_hash_path_and_closed_schema() {
    let valid = manifest();
    valid.validate("se").unwrap();
    for path in ["se", "../se", "/tmp/se", "mcp/runtimes/current/se"] {
        let mut changed = valid.clone();
        changed.runtime_file = path.into();
        assert_eq!(
            changed.validate("se").unwrap_err(),
            "runtime_manifest_path_invalid"
        );
    }
    let mut changed = valid.clone();
    changed.runtime_sha256 = "e".repeat(64);
    assert_eq!(
        changed.validate("se").unwrap_err(),
        "runtime_manifest_path_invalid"
    );
    for field in ["command", "args", "download_url", "credentials"] {
        let mut encoded = serde_json::to_value(&valid).unwrap();
        encoded[field] = serde_json::json!("not admitted");
        assert!(
            serde_json::from_value::<Manifest>(encoded).is_err(),
            "{field}"
        );
    }

    // Allowlist cases (the former allowlist test): duplicates and an unlisted starter.
    let mut duplicate_launcher = valid.clone();
    duplicate_launcher
        .allowed_launcher_hashes
        .push(duplicate_launcher.launcher_sha256.clone());
    let mut unlisted_starter = valid.clone();
    unlisted_starter.allowed_launcher_hashes = vec!["e".repeat(64)];
    let mut duplicate_contract = valid.clone();
    duplicate_contract
        .supported_runtimes
        .push(contract(&duplicate_contract.runtime_sha256));
    for (case, changed, expected) in [
        (
            "duplicate launcher hash",
            duplicate_launcher,
            "runtime_manifest_duplicate_or_invalid_launcher",
        ),
        (
            "unlisted starter",
            unlisted_starter,
            "runtime_manifest_current_launcher_not_allowed",
        ),
        (
            "duplicate contract",
            duplicate_contract,
            "runtime_manifest_duplicate_or_unimplemented_contract",
        ),
    ] {
        assert_eq!(changed.validate("se").unwrap_err(), expected, "{case}");
    }
}

#[test]
fn batches_p13_runtime_permission_requires_exact_hash_and_semantic_pair() {
    let mut policy = manifest();
    let admitted = contract(&"c".repeat(64));
    assert!(policy.permits(&admitted));
    assert!(policy.permits(&contract(&policy.runtime_sha256)));
    let mut changed = admitted.clone();
    changed.runtime_sha256 = "e".repeat(64);
    assert!(!policy.permits(&changed));
    changed = admitted.clone();
    changed.ipc_contract_id = "unimplemented-ipc".into();
    assert!(!policy.permits(&changed));
    policy.supported_runtimes.push(changed.clone());
    assert!(
        !policy.permits(&changed),
        "listing cannot implement a semantic adapter"
    );
    changed = admitted;
    changed.shared_state_contract_id = "unimplemented-state".into();
    policy.supported_runtimes.push(changed.clone());
    assert!(!policy.permits(&changed));
}

#[test]
fn batches_p13_probe_binds_catalog_content_version_and_protocols() {
    let version = env!("CARGO_PKG_VERSION");
    let mut probe = RuntimeProbe::current();
    probe.validate(version).unwrap();
    probe
        .compatibility_catalog
        .runtimes
        .push(contract(&"e".repeat(64)));
    assert!(
        probe.validate(version).is_err(),
        "catalog edit must invalidate its digest"
    );
    probe.compatibility_catalog_sha256 = probe.compatibility_catalog.sha256();
    probe.validate(version).unwrap();
    let approved = probe.compatibility_catalog.runtimes[0].clone();
    assert!(probe.compatibility_catalog.allows_runtime(&approved));
    let mut changed = approved;
    changed.shared_state_contract_id = "different-state".into();
    assert!(!probe.compatibility_catalog.allows_runtime(&changed));
    assert!(probe.validate("different-version").is_err());
    probe.lifecycle_protocol = 0;
    assert!(probe.validate(version).is_err());
}
