//! Same-volume moves must not require hardlink support or duplicate bytes.
use super::*;
use crate::types::CopyMode;
use std::{
    cell::Cell,
    fs,
    sync::atomic::{AtomicBool, Ordering},
};

thread_local! { static NO_HARDLINKS: Cell<bool> = const { Cell::new(false) }; }
thread_local! { static LOSE_MOVE_REPLY: Cell<bool> = const { Cell::new(false) }; }
pub(super) fn without_hardlinks() -> bool {
    NO_HARDLINKS.with(Cell::get)
}
pub(super) fn move_reply(result: io::Result<()>) -> io::Result<()> {
    result.and_then(|()| {
        if LOSE_MOVE_REPLY.with(|flag| flag.replace(false)) {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "move acknowledgement lost",
            ))
        } else {
            Ok(())
        }
    })
}
struct NoHardlinks;
impl NoHardlinks {
    fn new() -> Self {
        NO_HARDLINKS.with(|flag| {
            assert!(!flag.replace(true));
        });
        Self
    }
}
impl Drop for NoHardlinks {
    fn drop(&mut self) {
        NO_HARDLINKS.with(|flag| flag.set(false));
        LOSE_MOVE_REPLY.with(|flag| flag.set(false));
    }
}

fn request<'a>(
    source: &'a Path,
    target: &'a Path,
    cancel: &'a AtomicBool,
    conflict: Conflict,
) -> LocalRequest<'a> {
    LocalRequest {
        source,
        target,
        conflict,
        mode: CopyMode::Move,
        cancel,
        parallel: 1,
        probe: None,
    }
}

#[test]
fn batches_task_transfer_compat_move_without_hardlinks_does_not_copy_bytes() {
    let _capability = NoHardlinks::new();
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("from/file.bin");
    let target = root.path().join("to/file.bin");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(&source, b"same-volume payload").unwrap();
    let cancel = AtomicBool::new(false);
    let mut copied = 0;
    let outcome = super::super::transfer_local(
        &request(&source, &target, &cancel, Conflict::Rename),
        &mut |bytes| copied += bytes,
    )
    .unwrap();
    assert!(matches!(outcome, LocalOutcome::Completed { .. }));
    assert_eq!(
        copied, 0,
        "a rename must not enter the full stage-copy path"
    );
    assert!(!source.exists());
    assert_eq!(fs::read(&target).unwrap(), b"same-volume payload");
    assert_eq!(fs::read_dir(source.parent().unwrap()).unwrap().count(), 0);
    assert_eq!(fs::read_dir(target.parent().unwrap()).unwrap().count(), 1);
}

struct Observer<'a> {
    cancel: &'a AtomicBool,
    fail_after: bool,
    fail_before: bool,
    cancel_after_prepare: bool,
}
impl PublicationObserver for Observer<'_> {
    fn before_publish(&self, _: &Path, _: &Path) -> io::Result<()> {
        if self.fail_before {
            return Err(io::Error::other("injected preparation failure"));
        }
        if self.cancel_after_prepare {
            self.cancel.store(true, Ordering::Release);
        }
        Ok(())
    }
    fn after_publish(&self, _: &Path) -> io::Result<()> {
        if self.fail_after {
            Err(io::Error::other("injected receipt failure"))
        } else {
            Ok(())
        }
    }
    fn publication_preparation_failed(&self, _: &Path, _: &Path) -> io::Result<()> {
        Ok(())
    }
    fn publication_failed(&self, _: &Path, _: &Path) -> io::Result<()> {
        Ok(())
    }
    fn before_source_quarantine(&self, _: &Path, _: &Path) -> io::Result<()> {
        if !self.fail_before && !self.fail_after && !self.cancel_after_prepare {
            self.cancel.store(true, Ordering::Release);
        }
        Ok(())
    }
}

#[test]
fn batches_task_transfer_compat_move_without_hardlinks_retains_committed_target_on_receipt_failure()
{
    let _capability = NoHardlinks::new();
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::write(&source, b"kept").unwrap();
    let cancel = AtomicBool::new(false);
    let observer = Observer {
        cancel: &cancel,
        fail_after: true,
        fail_before: false,
        cancel_after_prepare: false,
    };
    let error = super::super::transfer_local_observed(
        &request(&source, &target, &cancel, Conflict::Rename),
        &mut |_| panic!("rename copied bytes"),
        &observer,
    )
    .unwrap_err()
    .into_error();
    assert!(error.to_string().contains("publication receipt failed"));
    assert_eq!(fs::read(&target).unwrap(), b"kept");
    assert!(!source.exists());
}

#[test]
fn batches_task_transfer_compat_move_without_hardlinks_keeps_unknown_committed_result() {
    let _capability = NoHardlinks::new();
    LOSE_MOVE_REPLY.with(|flag| flag.set(true));
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    fs::write(&source, b"retained at target").unwrap();
    let cancel = AtomicBool::new(false);
    let error = super::super::transfer_local(
        &request(&source, &target, &cancel, Conflict::Rename),
        &mut |_| panic!("rename copied bytes"),
    )
    .unwrap_err()
    .into_error();
    assert!(error
        .to_string()
        .contains("nonpublication authorization unconfirmed"));
    assert!(!error.to_string().contains("secured move source retained"));
    assert!(!source.exists());
    assert_eq!(fs::read(&target).unwrap(), b"retained at target");
}

#[test]
fn batches_task_transfer_compat_move_without_hardlinks_restores_failed_preparation_and_cancel() {
    let _capability = NoHardlinks::new();
    for (fail_before, cancel_after_prepare) in [(false, false), (false, true), (true, false)] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::write(&source, b"original").unwrap();
        let cancel = AtomicBool::new(false);
        let observer = Observer {
            cancel: &cancel,
            fail_after: false,
            fail_before,
            cancel_after_prepare,
        };
        let result = super::super::transfer_local_observed(
            &request(&source, &target, &cancel, Conflict::Rename),
            &mut |_| panic!("rename copied bytes"),
            &observer,
        );
        if fail_before {
            assert!(result.is_err());
        } else {
            assert!(matches!(result.unwrap(), LocalOutcome::Canceled));
        }
        assert_eq!(fs::read(&source).unwrap(), b"original");
        assert!(!target.exists());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
}

#[test]
fn batches_task_transfer_compat_move_without_hardlinks_preserves_conflicts() {
    let _capability = NoHardlinks::new();
    for conflict in [Conflict::Skip, Conflict::Rename, Conflict::Overwrite] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source.txt");
        let target = root.path().join("target.txt");
        fs::write(&source, b"ours").unwrap();
        fs::write(&target, b"foreign").unwrap();
        let cancel = AtomicBool::new(false);
        let outcome = super::super::transfer_local(
            &request(&source, &target, &cancel, conflict),
            &mut |_| panic!("rename copied bytes"),
        )
        .unwrap();
        match conflict {
            Conflict::Skip => {
                assert!(matches!(outcome, LocalOutcome::Skipped));
                assert_eq!(fs::read(&source).unwrap(), b"ours");
                assert_eq!(fs::read(&target).unwrap(), b"foreign");
            }
            Conflict::Rename => {
                assert!(!source.exists());
                assert_eq!(fs::read(&target).unwrap(), b"foreign");
                assert_eq!(
                    fs::read(root.path().join("target (2).txt")).unwrap(),
                    b"ours"
                );
            }
            Conflict::Overwrite => {
                assert!(!source.exists());
                assert_eq!(fs::read(&target).unwrap(), b"ours");
            }
            Conflict::Update => unreachable!("not in the list above"),
        }
    }
}
