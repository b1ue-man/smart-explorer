//! Real Windows access fixtures, executed only by the single remote task suite.
use crate::analytics::{scan, Progress, ScanStatus};
use crate::local_access::{
    access_fixtures::{
        acl, privileges, process_token, thread_token, token_id, DeniedDirectory, Identity,
    },
    parallel_scan_allowed, read_directory, BackupRead,
};
use std::{
    fs::OpenOptions, io, os::windows::fs::OpenOptionsExt, process::Command, sync::atomic::Ordering,
};

fn remote_only() {
    assert_eq!(
        std::env::var("SMART_EXPLORER_ANALYTICS_TASK").as_deref(),
        Ok("1"),
        "Use the checked-in remote analytics task entrypoint"
    );
}

#[test]
#[ignore = "real ACL and token fixture: remote Windows task only; Suite crates-task"]
fn analytics_access_task_real_denied_directory_locked_files_and_unchanged_acl() {
    remote_only();
    let process = process_token();
    let before = privileges(process.0);
    let previous = thread_token().map(|token| token_id(token.0));
    {
        let _backup = BackupRead::enable().expect("remote runner must own SeBackupPrivilege");
    }
    let fixture = tempfile::tempdir().unwrap();
    let protected = fixture.path().join("protected");
    std::fs::create_dir_all(protected.join("nested")).unwrap();
    std::fs::write(protected.join("locked.bin"), vec![0; 37]).unwrap();
    std::fs::write(protected.join("nested/note.md"), vec![0; 13]).unwrap();
    std::fs::write(fixture.path().join("sibling"), vec![0; 7]).unwrap();
    let _locked = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(protected.join("locked.bin"))
        .unwrap();
    assert!(std::fs::File::open(protected.join("locked.bin")).is_err());
    let denied = DeniedDirectory::new(&protected);
    denied.assert_ordinary_denied();
    let before_acl = acl(&protected);
    let progress = Progress::default();
    let outcome = scan(fixture.path(), &progress);
    assert_eq!(outcome.status, ScanStatus::Complete, "{:?}", outcome.issues);
    assert_eq!(outcome.tree.unwrap().size, 57);
    assert_eq!(progress.files.load(Ordering::Relaxed), 3);
    assert_eq!(outcome.permission_denied, 0);
    assert_eq!(
        acl(&protected),
        before_acl,
        "scanner must never rewrite ACLs"
    );
    assert_eq!(privileges(process.0), before, "process privileges changed");
    assert_eq!(thread_token().map(|token| token_id(token.0)), previous);
    // The root-denied route must also work, not only child-directory retries.
    let outcome = scan(&protected, &Progress::default());
    assert_eq!(outcome.status, ScanStatus::Complete, "{:?}", outcome.issues);
    assert_eq!(outcome.tree.unwrap().size, 50);
}

#[test]
#[ignore = "thread impersonation fixture: remote Windows task only; Suite crates-task"]
fn analytics_access_task_restricted_identity_never_falls_back_to_process_authority() {
    remote_only();
    let fixture = tempfile::tempdir().unwrap();
    let protected = fixture.path().join("protected");
    std::fs::create_dir(&protected).unwrap();
    std::fs::write(protected.join("file"), "secret").unwrap();
    let other = fixture.path().join("protected_other");
    std::fs::create_dir(&other).unwrap();
    std::fs::write(other.join("file"), "other").unwrap();
    std::fs::write(fixture.path().join("public.bin"), vec![0; 7]).unwrap();
    let _denied = DeniedDirectory::new(&protected);
    let _other_denied = DeniedDirectory::new(&other);
    {
        let identity = Identity::new(false);
        let previous_id = token_id(identity._token.0);
        let previous_privileges = privileges(identity._token.0);
        assert!(!parallel_scan_allowed());
        {
            let _backup = BackupRead::enable().unwrap();
        }
        assert_eq!(token_id(thread_token().unwrap().0), previous_id);
        let outcome = scan(fixture.path(), &Progress::default());
        assert_eq!(outcome.status, ScanStatus::Complete, "{:?}", outcome.issues);
        assert_eq!(outcome.tree.unwrap().size, 18);
        assert_eq!(token_id(thread_token().unwrap().0), previous_id);
        assert_eq!(privileges(identity._token.0), previous_privileges);
    }
    {
        let identity = Identity::new(true);
        let previous_id = token_id(identity._token.0);
        let previous_privileges = privileges(identity._token.0);
        assert!(!parallel_scan_allowed());
        assert!(BackupRead::enable().is_err());
        let error = read_directory(&protected)
            .err()
            .expect("restricted token must not bypass denial");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(token_id(thread_token().unwrap().0), previous_id);
        // Multiple directories exercise recursive dispatch too: selecting one
        // scan thread must not escape to Rayon's global worker pool here.
        let outcome = scan(fixture.path(), &Progress::default());
        assert_eq!(outcome.status, ScanStatus::Partial, "{:?}", outcome.issues);
        assert_eq!(outcome.permission_denied, 2);
        assert_eq!(outcome.tree.unwrap().size, 7);
        let outcome = scan(&protected, &Progress::default());
        assert_eq!(outcome.status, ScanStatus::Failed, "{:?}", outcome.issues);
        assert_eq!(outcome.permission_denied, 1);
        assert!(outcome.tree.is_none());
        assert_eq!(token_id(thread_token().unwrap().0), previous_id);
        assert_eq!(privileges(identity._token.0), previous_privileges);
    }
}

#[test]
#[ignore = "real junction fixture: remote Windows task only; Suite crates-task"]
fn analytics_access_task_redirect_children_are_traversal_boundaries() {
    remote_only();
    let fixture = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("outside.bin"), vec![0; 100]).unwrap();
    std::fs::write(fixture.path().join("inside.bin"), vec![0; 7]).unwrap();
    let link = fixture.path().join("junction");
    let created = Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&link)
        .arg(outside.path())
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let outcome = scan(fixture.path(), &Progress::default());
    // Remove the junction itself, never recursively traverse its target.
    std::fs::remove_dir(&link).unwrap();
    assert_eq!(outcome.status, ScanStatus::Complete, "{:?}", outcome.issues);
    assert_eq!(outcome.tree.unwrap().size, 7);
    assert_eq!(
        std::fs::read(outside.path().join("outside.bin"))
            .unwrap()
            .len(),
        100
    );
}
