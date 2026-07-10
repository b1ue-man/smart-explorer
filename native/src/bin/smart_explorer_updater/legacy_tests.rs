use super::*;

fn args_for(target: &Path, staged: &Path, dir: &Path) -> LegacyApplyArgs {
    LegacyApplyArgs {
        target: target.to_path_buf(),
        staged: staged.to_path_buf(),
        staged_sha256: sha256_file(staged).unwrap(),
        helper_sha256: None,
        parent_pid: 0,
        version: "0.5.121".to_string(),
        last_applied: dir.join("last.txt"),
        error_file: dir.join("error.txt"),
    }
}

#[test]
fn tampered_legacy_staging_is_rejected_before_target_changes() {
    let dir = tempfile::tempdir().unwrap();
    let staged = dir.path().join("staged");
    let target = dir.path().join("target");
    std::fs::write(&staged, b"new-app").unwrap();
    std::fs::write(&target, b"old-app").unwrap();
    let staged_sha256 = sha256_file(&staged).unwrap();
    std::fs::write(&staged, b"bad-app").unwrap();
    let mut args = args_for(&target, &staged, dir.path());
    args.staged_sha256 = staged_sha256;

    assert!(verify_inputs(&args, Path::new("unused-helper")).is_err());
    assert_eq!(std::fs::read(target).unwrap(), b"old-app");
}

#[test]
fn duplicate_worker_accepts_matching_completed_version_idempotently() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    let staged = dir.path().join("staged");
    std::fs::write(&target, b"new-app").unwrap();
    std::fs::write(&staged, b"new-app").unwrap();
    let args = args_for(&target, &staged, dir.path());
    std::fs::write(&args.last_applied, &args.version).unwrap();
    let target_key = instance::target_key(&target);
    let marker = launch_complete_path(&args.last_applied, &target_key).unwrap();
    write_launch_complete(&marker, &target_key, &args.version, &args.staged_sha256).unwrap();

    recover_or_accept_completed_update(&args).unwrap();

    assert!(!staged.exists());
    assert_eq!(
        std::fs::read_to_string(args.last_applied).unwrap(),
        "0.5.121"
    );
}

#[test]
fn serialized_winner_retires_old_request_and_rebases_newer_request() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    let staged = dir.path().join("staged");
    std::fs::write(&target, b"winner").unwrap();
    std::fs::write(&staged, b"request").unwrap();
    let args = args_for(&target, &staged, dir.path());
    let current_sha256 = sha256_file(&target).unwrap();
    std::fs::write(&args.last_applied, b"0.5.120").unwrap();
    let target_key = instance::target_key(&target);
    let marker = launch_complete_path(&args.last_applied, &target_key).unwrap();
    write_launch_complete(&marker, &target_key, "0.5.120", &current_sha256).unwrap();

    let (winner, winner_number) = completed_winner(&args, &current_sha256).unwrap().unwrap();
    assert_eq!(winner, "0.5.120");
    // This models a stale worker first scheduled only after the completed
    // winner was already its baseline: it must still retire, never downgrade.
    assert!(parse_release_version("0.5.119").unwrap() <= winner_number);
    assert!(parse_release_version("0.5.121").unwrap() > winner_number);
    assert_eq!(parse_release_version("1.10.0").unwrap(), (1, 10, 0));
    assert!(parse_release_version("1.02.0").is_err());

    std::fs::remove_file(marker).unwrap();
    assert!(completed_winner(&args, &current_sha256).unwrap().is_none());
}

#[test]
fn unproven_newer_status_blocks_a_late_stale_worker() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    let staged = dir.path().join("staged");
    std::fs::write(&target, b"newer-winner").unwrap();
    std::fs::write(&staged, b"stale-request").unwrap();
    let args = args_for(&target, &staged, dir.path());
    std::fs::write(&args.last_applied, b"0.5.122").unwrap();

    assert!(apply_update(args).is_err());
    assert_eq!(std::fs::read(target).unwrap(), b"newer-winner");
}

#[test]
fn durable_intent_blocks_a_crash_before_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    let staged = dir.path().join("staged");
    std::fs::write(&target, b"installed-baseline").unwrap();
    std::fs::write(&staged, b"stale-request").unwrap();
    let args = args_for(&target, &staged, dir.path());
    let baseline = sha256_file(&target).unwrap();
    let intent = LegacyIntent::create(&target, "0.5.122", &baseline, &"a".repeat(64)).unwrap();

    assert!(apply_update(args).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"installed-baseline");
    assert!(LegacyIntent::load(&target).unwrap().is_some());
    intent.clear().unwrap();
}

#[cfg(unix)]
#[test]
fn durable_intent_prevents_downgrade_after_replace_before_status_crash() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    let staged = dir.path().join("staged");
    std::fs::write(&target, b"installed-baseline").unwrap();
    std::fs::write(&staged, b"stale-request").unwrap();
    let baseline = sha256_file(&target).unwrap();
    let winner = b"newer-winner";
    let winner_sha256 = {
        let winner_file = dir.path().join("winner");
        std::fs::write(&winner_file, winner).unwrap();
        sha256_file(&winner_file).unwrap()
    };
    let intent = LegacyIntent::create(&target, "0.5.122", &baseline, &winner_sha256).unwrap();
    std::fs::write(&target, winner).unwrap();
    let args = args_for(&target, &staged, dir.path());

    assert!(apply_update(args).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), winner);
    assert!(LegacyIntent::load(&target).unwrap().is_some());
    intent.clear().unwrap();
}

#[cfg(unix)]
fn write_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(path, permissions).unwrap();
}

#[cfg(unix)]
fn wait_for_file(path: &Path) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !path.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(unix)]
#[test]
fn missing_completion_receipt_forces_verified_relaunch() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.sh");
    let staged = dir.path().join("staged.sh");
    let observed = dir.path().join("observed.txt");
    let last = dir.path().join("last.txt");
    let error = dir.path().join("error.txt");
    let script = format!(
        "#!/bin/bash\nprintf '%s' \"$SMART_EXPLORER_UPDATE_ACK_PAYLOAD\" > \"$SMART_EXPLORER_UPDATE_ACK_PATH\"\nhost=${{SMART_EXPLORER_UPDATE_ACK_SIGNAL%:*}}; port=${{SMART_EXPLORER_UPDATE_ACK_SIGNAL##*:}}; exec 3<>\"/dev/tcp/$host/$port\"; printf '%s' \"$SMART_EXPLORER_UPDATE_ACK_TOKEN\" >&3; dd bs=1 count=1 <&3 >/dev/null 2>&1\nif [ \"$(cat '{}')\" = \"0.5.121\" ] && [ ! -e '{}' ]; then printf ok > '{}'; else printf bad > '{}'; fi\nsleep 0.2\n",
        last.display(),
        error.display(),
        observed.display(),
        observed.display()
    );
    write_executable(&target, &script);
    std::fs::copy(&target, &staged).unwrap();
    std::fs::write(&last, b"0.5.121").unwrap();
    std::fs::write(&error, b"stale").unwrap();
    let args = args_for(&target, &staged, dir.path());

    recover_or_accept_completed_update(&args).unwrap();
    wait_for_file(&observed);

    assert_eq!(std::fs::read_to_string(observed).unwrap(), "ok");
    assert_eq!(std::fs::read_to_string(last).unwrap(), "0.5.121");
    assert!(!error.exists());
    assert!(!staged.exists());
    let target_key = instance::target_key(&target);
    let marker = launch_complete_path(&args.last_applied, &target_key).unwrap();
    assert!(
        launch_complete_matches(&marker, &target_key, &args.version, &args.staged_sha256).unwrap()
    );
}
