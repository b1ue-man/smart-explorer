//! Program-file removal fixtures with isolated roots and actual file locks.
//! The Linux handoff case alone runs the runner's candidate CLI in its own
//! copied installation; no case uses a real host profile or an installer.
use super::*;
use crate::mcp_launcher::installation::atomic_json;

struct Fixture {
    _directory: tempfile::TempDir,
    installation: Installation,
}
impl Fixture {
    fn new() -> Self {
        Self::new_at(None)
    }

    fn new_at(parent: Option<&std::path::Path>) -> Self {
        let directory = match parent {
            Some(parent) => tempfile::tempdir_in(parent).unwrap(),
            None => tempfile::tempdir().unwrap(),
        };
        let root = std::fs::canonicalize(directory.path()).unwrap();
        os::protect(&root, true).unwrap();
        let runtime = b"\x7fELF-regression-runtime";
        let launcher = b"\x7fELF-regression-launcher";
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
            ipc_contract_id: crate::mcp_launcher::IPC_CONTRACT.into(),
            shared_state_contract_id: crate::mcp_launcher::STATE_CONTRACT.into(),
            supported_runtimes: vec![],
            compatibility_catalog_sha256: "a".repeat(64),
        };
        for name in [
            "mcp".into(),
            "mcp/runtimes".into(),
            format!("mcp/runtimes/{runtime_hash}"),
            "mcp/launchers".into(),
            format!("mcp/launchers/{launcher_hash}"),
        ] {
            let path = root.join(name);
            std::fs::create_dir(&path).unwrap();
            os::protect(&path, true).unwrap();
        }
        let write = |name: &str, bytes: &[u8], executable| {
            let path = root.join(name);
            std::fs::write(&path, bytes).unwrap();
            os::protect(&path, executable).unwrap();
        };
        write(os::CLI_NAME, runtime, true);
        write(&manifest.runtime_file, runtime, true);
        write(
            &format!("mcp/launchers/{launcher_hash}/{}", os::LAUNCHER_NAME),
            launcher,
            true,
        );
        write(
            &format!("mcp/launchers/{launcher_hash}/info.v1.json"),
            br#"{"product_version":"1.0.0","launcher_protocol":1}"#,
            false,
        );
        write(
            &format!("mcp/runtimes/{runtime_hash}/lifetime.lock"),
            b"",
            false,
        );
        write(
            "mcp/installation.id",
            manifest.installation_id.as_bytes(),
            false,
        );
        write("mcp/install-last-applied.txt", b"0.9.0", false);
        for name in ["update-owner.lock", "control.lock", "operations.lock"] {
            write(&format!("mcp/{name}"), b"", false);
        }
        atomic_json(&root.join("mcp/current.v1.json"), &manifest).unwrap();
        let installation = Installation::load(&root).unwrap();
        Self {
            _directory: directory,
            installation,
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn batches_p13_linux_installed_cli_retirement_reexecs_only_with_its_bound_root() {
    let source = std::env::var_os("SE_SCREEN_TASK_WORKER")
        .expect("the existing remote task runner must provide its candidate se binary");
    let source = std::fs::canonicalize(source).unwrap();
    assert!(source.is_file(), "candidate se binary missing");
    // An executable installation is staged on the candidate's own filesystem.
    // The child's system temp below is deliberately unusable for execution.
    let fixture = Fixture::new_at(Some(source.parent().unwrap()));
    let root = &fixture.installation.root;
    let hash = crate::updater::file_sha256(&source).unwrap();
    let mut manifest = fixture.installation.manifest.clone();
    manifest.runtime_sha256 = hash.clone();
    manifest.runtime_file = format!("mcp/runtimes/{hash}/{}", os::CLI_NAME);
    let runtime_directory = root.join("mcp/runtimes").join(&hash);
    std::fs::create_dir(&runtime_directory).unwrap();
    os::protect(&runtime_directory, true).unwrap();
    std::fs::copy(&source, root.join(os::CLI_NAME)).unwrap();
    std::fs::copy(&source, root.join(&manifest.runtime_file)).unwrap();
    std::fs::write(runtime_directory.join("lifetime.lock"), b"").unwrap();
    atomic_json(&root.join("mcp/current.v1.json"), &manifest).unwrap();
    let user = root.join("mcp/user-notes.txt");
    std::fs::write(&user, b"keep user data").unwrap();
    let rejected = std::process::Command::new(root.join(os::CLI_NAME))
        .args(["plugin", "retire-installation"])
        .env("SE_MCP_RETIREMENT_ROOT_FD", "1").env_remove("COMPLETE")
        .output().unwrap();
    assert!(!rejected.status.success(), "forged root descriptor was accepted");
    assert!(root.join("mcp/current.v1.json").exists());
    assert!(root.join(&manifest.runtime_file).exists());
    let output = std::process::Command::new(root.join(os::CLI_NAME))
        .args(["plugin", "retire-installation"])
        .env("TMPDIR", "/proc").env_remove("SE_MCP_RETIREMENT_ROOT_FD")
        .env_remove("COMPLETE").output().unwrap();
    assert!(output.status.success(), "retirement failed: {}", String::from_utf8_lossy(&output.stderr));
    assert!(!root.join("mcp/current.v1.json").exists());
    assert!(!root.join(&manifest.runtime_file).exists());
    assert!(root.join(os::CLI_NAME).exists(), "ordinary installed CLI belongs to the installer stage");
    assert_eq!(std::fs::read(&user).unwrap(), b"keep user data");
    assert!(!std::fs::read_dir(root).unwrap().any(|entry|
        entry.unwrap().file_name().to_string_lossy().starts_with(".mcp-retire-")),
        "retirement left its own staging program behind");
}

#[test]
fn batches_p13_uninstall_removes_all_owned_mcp_programs_and_manifest() {
    let fixture = Fixture::new();
    let installation = &fixture.installation;
    let root = &installation.root;
    // A superseded self-installed generation is still part of the product.
    let bytes = b"\x7fELF-previous-regression-runtime";
    let hash = hash_bytes(bytes);
    let directory = root.join("mcp/runtimes").join(hash);
    std::fs::create_dir(&directory).unwrap();
    os::protect(&directory, true).unwrap();
    for (name, bytes, executable) in [
        (os::CLI_NAME, bytes.as_slice(), true),
        ("lifetime.lock", b"".as_slice(), false),
    ] {
        let path = directory.join(name);
        std::fs::write(&path, bytes).unwrap();
        os::protect(&path, executable).unwrap();
    }
    retire_mcp(root, None).unwrap();
    assert!(!root.join("mcp").exists());
    assert!(
        installation.installed_cli().exists(),
        "NSIS removes the ordinary CLI only after this stage"
    );
    assert!(Installation::for_image(&installation.installed_cli())
        .unwrap()
        .is_none());
}

#[test]
fn batches_p13_uninstall_preserves_foreign_files_and_is_repeatable() {
    let fixture = Fixture::new();
    let installation = &fixture.installation;
    let root = &installation.root;
    let user = root.join("mcp/user-notes.txt");
    std::fs::write(&user, b"user data").unwrap();
    std::fs::write(
        root.join("mcp/install-last-applied.txt"),
        b"user repurposed this file",
    )
    .unwrap();
    let nested = installation
        .runtime_path()
        .parent()
        .unwrap()
        .join("user-notes.txt");
    std::fs::write(&nested, b"nested user data").unwrap();
    let foreign = root.join("mcp/runtimes/user-directory");
    std::fs::create_dir(&foreign).unwrap();
    std::fs::write(foreign.join("keep.txt"), b"foreign").unwrap();
    retire_mcp(root, None).unwrap();
    assert!(!installation.runtime_path().exists());
    assert!(!installation.launcher_path().exists());
    assert!(!root.join("mcp/current.v1.json").exists());
    assert_eq!(std::fs::read(user).unwrap(), b"user data");
    assert_eq!(std::fs::read(nested).unwrap(), b"nested user data");
    assert_eq!(
        std::fs::read(root.join("mcp/install-last-applied.txt")).unwrap(),
        b"user repurposed this file"
    );
    assert_eq!(std::fs::read(foreign.join("keep.txt")).unwrap(), b"foreign");
    assert!(
        !remaining_managed_files(root).unwrap(),
        "a later NSIS retry must accept only foreign leftovers"
    );
}

#[test]
fn batches_p13_uninstall_respects_independent_owner_operation_and_lifetime_locks() {
    for name in [
        "update-owner.lock".into(),
        "control.lock".into(),
        "operations.lock".into(),
        format!(
            "runtimes/{}/lifetime.lock",
            Fixture::new().installation.manifest.runtime_sha256
        ),
    ] {
        let fixture = Fixture::new();
        let installation = &fixture.installation;
        let root = &installation.root;
        let held = installation.lock(&name, false).unwrap();
        assert!(
            retire_mcp(root, None)
                .unwrap_err()
                .starts_with("lock_busy:"),
            "{name}"
        );
        assert!(installation.runtime_path().exists());
        assert!(installation.launcher_path().exists());
        assert!(root.join("mcp/current.v1.json").exists());
        assert!(!root.join("mcp/uninstall.v1.json").exists());
        drop(held);
        retire_mcp(root, None).unwrap();
        assert!(!root.join("mcp").exists());
    }
}

#[test]
fn batches_p13_uninstall_rejects_changed_program_metadata_and_owner_before_deletion() {
    for which in ["runtime", "launcher", "identity", "metadata"] {
        let fixture = Fixture::new();
        let installation = &fixture.installation;
        let root = &installation.root;
        let path = match which {
            "runtime" => installation.runtime_path(),
            "launcher" => installation.launcher_path(),
            "identity" => root.join("mcp/installation.id"),
            _ => installation
                .launcher_path()
                .parent()
                .unwrap()
                .join("info.v1.json"),
        };
        std::fs::write(&path, b"user-edited content").unwrap();
        assert!(retire_mcp(root, None).is_err(), "{which}");
        assert_eq!(std::fs::read(path).unwrap(), b"user-edited content");
        assert!(root.join("mcp/current.v1.json").exists());
        assert!(installation.runtime_path().exists());
        assert!(installation.launcher_path().exists());
        assert!(!root.join("mcp/uninstall.v1.json").exists());
    }
}

#[test]
fn batches_p13_uninstall_preserves_open_update_recovery_and_blocks_new_admission() {
    let fixture = Fixture::new();
    let installation = &fixture.installation;
    let root = &installation.root;
    let transaction = crate::mcp_launcher::UpdateTransaction::prepare_retirement(
        root,
        "1.0.1",
        vec![crate::mcp_launcher::core::lifecycle::InstallationTarget {
            file: os::CLI_NAME.into(),
            previous_sha256: Some(installation.manifest.runtime_sha256.clone()),
            target_sha256: installation.manifest.runtime_sha256.clone(),
        }],
    )
    .unwrap();
    drop(transaction);
    assert_eq!(
        retire_mcp(root, None).unwrap_err(),
        "uninstall_requires_update_recovery"
    );
    assert!(installation.runtime_path().exists());
    assert!(root.join("mcp/update.v1.json").exists());
    let mut state: crate::mcp_launcher::core::lifecycle::UpdateState =
        read_json(&root.join("mcp/update.v1.json")).unwrap();
    state.phase = crate::mcp_launcher::core::lifecycle::Phase::RolledBack;
    atomic_json(&root.join("mcp/update.v1.json"), &state).unwrap();
    let record = inventory(installation).unwrap();
    atomic_json(&root.join("mcp/uninstall.v1.json"), &record).unwrap();
    assert_eq!(
        crate::mcp_launcher::admission::check_state(installation).unwrap_err(),
        "installation_uninstall_in_progress"
    );
    assert_eq!(
        Installation::for_image(&installation.installed_cli()).unwrap_err(),
        "installation_uninstall_in_progress"
    );
    assert_eq!(
        crate::mcp_launcher::generator::verified_legacy_entry(
            &installation.installed_cli(),
            &installation.manifest.runtime_sha256
        )
        .unwrap_err(),
        "installation_uninstall_in_progress"
    );
}

#[test]
fn batches_p13_uninstall_resumes_exact_record_after_partial_deletion_and_keeps_new_bytes() {
    let fixture = Fixture::new();
    let installation = &fixture.installation;
    let root = &installation.root;
    let record = inventory(installation).unwrap();
    atomic_json(&root.join("mcp/uninstall.v1.json"), &record).unwrap();
    std::fs::remove_file(root.join("mcp/current.v1.json")).unwrap();
    std::fs::remove_file(installation.runtime_path()).unwrap();
    let launcher = installation.launcher_path();
    let original = std::fs::read(&launcher).unwrap();
    std::fs::write(&launcher, b"new user content").unwrap();
    assert!(retire_mcp(root, load_removal(root).unwrap())
        .unwrap_err()
        .contains("uninstall_content_changed"));
    assert_eq!(std::fs::read(&launcher).unwrap(), b"new user content");
    assert!(root.join("mcp/uninstall.v1.json").exists());
    std::fs::write(&launcher, original).unwrap();
    retire_mcp(root, load_removal(root).unwrap()).unwrap();
    assert!(!root.join("mcp").exists());
}

#[test]
fn batches_p13_uninstall_record_never_authorizes_path_traversal() {
    let fixture = Fixture::new();
    let installation = &fixture.installation;
    let root = &installation.root;
    let user = root.join("keep.txt");
    std::fs::write(&user, b"user").unwrap();
    let mut record = inventory(installation).unwrap();
    record.files.push(RemovalFile {
        path: "mcp/../keep.txt".into(),
        sha256: hash_bytes(b"user"),
    });
    atomic_json(&root.join("mcp/uninstall.v1.json"), &record).unwrap();
    assert!(load_removal(root).is_err());
    assert_eq!(std::fs::read(user).unwrap(), b"user");
    assert!(installation.runtime_path().exists());
}

#[test]
fn batches_p13_uninstall_missing_lock_never_releases_remaining_program_files() {
    let fixture = Fixture::new();
    let installation = &fixture.installation;
    let root = &installation.root;
    let record = inventory(installation).unwrap();
    atomic_json(&root.join("mcp/uninstall.v1.json"), &record).unwrap();
    std::fs::remove_file(root.join("mcp/operations.lock")).unwrap();
    assert_eq!(
        retire_mcp(root, load_removal(root).unwrap()).unwrap_err(),
        "uninstall_missing_locks_with_executable_owners_unproved"
    );
    assert!(installation.runtime_path().exists());
    assert!(installation.launcher_path().exists());
    // A crash after executable deletion may legitimately leave no locks.
    std::fs::remove_file(installation.runtime_path()).unwrap();
    std::fs::remove_file(installation.launcher_path()).unwrap();
    retire_mcp(root, load_removal(root).unwrap()).unwrap();
    assert!(!root.join("mcp").exists());
}

#[test]
fn batches_p13_uninstall_oversized_inventory_is_rejected_before_publication() {
    let fixture = Fixture::new();
    let installation = &fixture.installation;
    let root = &installation.root;
    let mut record = inventory(installation).unwrap();
    for index in 0..2000 {
        let hash = format!("{index:064x}");
        record.files.push(RemovalFile {
            path: format!("mcp/runtimes/{hash}/{}", os::CLI_NAME),
            sha256: hash,
        });
    }
    assert!(write_removal_record(root, &record)
        .unwrap_err()
        .starts_with("uninstall_record_exceeds_record_budget:"));
    assert!(!root.join("mcp/uninstall.v1.json").exists());
    assert!(installation.runtime_path().exists());
    assert!(root.join("mcp/current.v1.json").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn batches_p13_uninstall_does_not_follow_foreign_child_links() {
    let fixture = Fixture::new();
    let root = &fixture.installation.root;
    let foreign = tempfile::tempdir().unwrap();
    std::fs::write(foreign.path().join("keep.txt"), b"foreign").unwrap();
    let link = fixture
        .installation
        .runtime_path()
        .parent()
        .unwrap()
        .join("user-link");
    std::os::unix::fs::symlink(foreign.path(), &link).unwrap();
    retire_mcp(root, None).unwrap();
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(
        std::fs::read(foreign.path().join("keep.txt")).unwrap(),
        b"foreign"
    );
}
