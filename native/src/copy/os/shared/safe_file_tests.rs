use super::*;

fn base(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("se_copy_{name}_{}", std::process::id()))
}

fn transfer(
    source: &Path,
    target: &Path,
    root: &Path,
    conflict: Conflict,
    mode: CopyMode,
    cancel: &AtomicBool,
) -> io::Result<TransferResult> {
    transfer_file(source, target, root, conflict, mode, cancel)
}

#[test]
fn overwrite_self_is_rejected_without_truncation() {
    let base = base("self");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let file = base.join("file.txt");
    std::fs::write(&file, b"keep").unwrap();
    let result = transfer(
        &file,
        &file,
        &base,
        Conflict::Overwrite,
        CopyMode::Copy,
        &AtomicBool::new(false),
    );
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    assert_eq!(std::fs::read(&file).unwrap(), b"keep");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn overwrite_hard_link_alias_is_rejected_without_data_loss() {
    let base = base("alias");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let source = base.join("source.txt");
    let alias = base.join("alias.txt");
    std::fs::write(&source, b"keep").unwrap();
    std::fs::hard_link(&source, &alias).unwrap();
    let result = transfer(
        &source,
        &alias,
        &base,
        Conflict::Overwrite,
        CopyMode::Copy,
        &AtomicBool::new(false),
    );
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    assert_eq!(std::fs::read(&source).unwrap(), b"keep");
    assert_eq!(std::fs::read(&alias).unwrap(), b"keep");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn canceled_overwrite_keeps_existing_destination() {
    let base = base("cancel");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let source = base.join("source.txt");
    let target = base.join("target.txt");
    std::fs::write(&source, b"new").unwrap();
    std::fs::write(&target, b"old").unwrap();
    let result = transfer(
        &source,
        &target,
        &base,
        Conflict::Overwrite,
        CopyMode::Copy,
        &AtomicBool::new(true),
    )
    .unwrap();
    assert!(matches!(result, TransferResult::Canceled));
    assert_eq!(std::fs::read(&target).unwrap(), b"old");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn no_replace_commit_preserves_intervening_destination() {
    let base = base("race");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let staged = base.join("staged");
    let target = base.join("target");
    std::fs::write(&staged, b"new").unwrap();
    std::fs::write(&target, b"intervening").unwrap();
    let result = platform::commit_staged(&staged, &target, false);
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&target).unwrap(), b"intervening");
    assert_eq!(std::fs::read(&staged).unwrap(), b"new");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn changed_move_source_is_not_deleted() {
    let base = base("source_swap");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let source = base.join("source");
    std::fs::write(&source, b"original").unwrap();
    let secured = quarantine_source(&source).unwrap();
    std::fs::write(&source, b"replacement").unwrap();

    let error = restore_quarantine(&secured).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(std::fs::read(&source).unwrap(), b"replacement");
    assert_eq!(std::fs::read(&secured.path).unwrap(), b"original");
    let _ = std::fs::remove_dir_all(&base);
}

#[cfg(unix)]
#[test]
fn destination_symlink_ancestor_is_rejected() {
    use std::os::unix::fs::symlink;

    let base = base("escape");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("root")).unwrap();
    std::fs::create_dir_all(base.join("outside")).unwrap();
    symlink(base.join("outside"), base.join("root/link")).unwrap();
    let source = base.join("source");
    std::fs::write(&source, b"data").unwrap();
    let target = base.join("root/link/victim");
    let result = transfer(
        &source,
        &target,
        &base.join("root"),
        Conflict::Overwrite,
        CopyMode::Copy,
        &AtomicBool::new(false),
    );
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    assert!(!base.join("outside/victim").exists());
    let _ = std::fs::remove_dir_all(&base);
}
