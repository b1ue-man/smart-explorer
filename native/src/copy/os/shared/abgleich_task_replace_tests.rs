//! A replacement the file rule decided is bound to the target it saw (review
//! B1): right before the stage replaces the target (the staged publication and
//! the direct move of a same-volume move), size and time are compared with the
//! stamp of the decision. A target saved again meanwhile is kept and the copy
//! is numbered beside it; an unchanged one is replaced as before.
use super::*;
use crate::transfer::update_rule::{Replacing, Stamp};
use std::time::{Duration, UNIX_EPOCH};

fn folder(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("se_abgleich_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn write_at(path: &Path, bytes: &[u8], seconds: u64) {
    std::fs::write(path, bytes).unwrap();
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(seconds))
        .unwrap();
}

/// What the rule saw of `target` when it decided to replace it.
fn seen(target: &Path) -> Replacing {
    let meta = std::fs::metadata(target).unwrap();
    Replacing {
        id: None,
        stamp: Stamp {
            size: meta.len(),
            mtime_ms: crate::local_access::system_time_ms(meta.modified().unwrap()),
        },
        window_ms: 2_000,
    }
}

fn replace(source: &Path, target: &Path, mode: CopyMode, decided: &Replacing) -> LocalOutcome {
    let cancel = AtomicBool::new(false);
    let request = LocalRequest {
        source,
        target,
        conflict: Conflict::Overwrite,
        mode,
        cancel: &cancel,
        parallel: 1,
        probe: None,
    };
    transfer_local_inner(&request, Start::Look, &mut |_| {}, None, Some(decided)).unwrap()
}

fn published(outcome: LocalOutcome) -> PathBuf {
    match outcome {
        LocalOutcome::Completed { target, .. } => target,
        other => panic!("expected a publication, got {other:?}"),
    }
}

#[test]
fn abgleich_task_an_unchanged_local_target_is_replaced() {
    let dir = folder("unchanged");
    let (source, target) = (dir.join("source"), dir.join("target"));
    write_at(&source, b"newer content", 6_000);
    write_at(&target, b"old", 1_000);
    let decided = seen(&target);
    let at = published(replace(&source, &target, CopyMode::Copy, &decided));
    assert_eq!(at, target, "published under the target's own name");
    assert_eq!(std::fs::read(&target).unwrap(), b"newer content");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn abgleich_task_a_local_target_saved_again_meanwhile_is_kept_and_the_copy_numbered() {
    let dir = folder("saved");
    let (source, target) = (dir.join("source.txt"), dir.join("target.txt"));
    write_at(&source, b"newer content", 6_000);
    write_at(&target, b"old", 1_000);
    let decided = seen(&target);
    // The person saves the target again after the decision.
    write_at(&target, b"saved meanwhile", 9_000);
    let at = published(replace(&source, &target, CopyMode::Copy, &decided));
    assert_eq!(at, dir.join("target (2).txt"));
    assert_eq!(std::fs::read(&target).unwrap(), b"saved meanwhile");
    assert_eq!(std::fs::read(&at).unwrap(), b"newer content");
    assert_eq!(std::fs::read(&source).unwrap(), b"newer content");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn abgleich_task_a_move_over_a_target_saved_again_numbers_the_moved_file() {
    let dir = folder("move");
    let (source, target) = (dir.join("source.txt"), dir.join("target.txt"));
    write_at(&source, b"newer content", 6_000);
    write_at(&target, b"old", 1_000);
    let decided = seen(&target);
    write_at(&target, b"saved meanwhile", 9_000);
    let at = published(replace(&source, &target, CopyMode::Move, &decided));
    assert_eq!(at, dir.join("target (2).txt"));
    assert_eq!(std::fs::read(&target).unwrap(), b"saved meanwhile");
    assert_eq!(std::fs::read(&at).unwrap(), b"newer content");
    assert!(!source.exists(), "a move takes the source away");
    let _ = std::fs::remove_dir_all(&dir);
}
