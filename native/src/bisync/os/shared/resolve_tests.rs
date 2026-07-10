use super::*;
use crate::vfs::{Backend, LocalBackend};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "bisync_resolve_{tag}_{}_{}",
        std::process::id(),
        nanos
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn forward(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn signature(backend: &dyn Backend, path: &str) -> Sig {
    let metadata = backend.stat(path).unwrap();
    Sig {
        size: metadata.size,
        mtime_ms: metadata.mtime_ms,
        hash: 0,
    }
}

fn fixture(tag: &str, a_bytes: Option<&[u8]>, b_bytes: Option<&[u8]>) -> ResolveFixture {
    let a_dir = temp_dir(&format!("{tag}_a"));
    let b_dir = temp_dir(&format!("{tag}_b"));
    let root_a = forward(&a_dir);
    let root_b = forward(&b_dir);
    let backend_a = LocalBackend::new(&root_a);
    let backend_b = LocalBackend::new(&root_b);
    let path_a = format!("{root_a}/f.txt");
    let path_b = format!("{root_b}/f.txt");
    if let Some(bytes) = a_bytes {
        std::fs::write(&path_a, bytes).unwrap();
    }
    if let Some(bytes) = b_bytes {
        std::fs::write(&path_b, bytes).unwrap();
    }
    let conflict = Conflict {
        rel: "f.txt".into(),
        a: a_bytes.map(|_| signature(&backend_a, &path_a)),
        b: b_bytes.map(|_| signature(&backend_b, &path_b)),
    };
    let pair = format!("resolve-test-{}-{tag}", std::process::id());
    ResolveFixture {
        a_dir,
        b_dir,
        root_a,
        root_b,
        backend_a,
        backend_b,
        path_a,
        path_b,
        pair,
        conflict,
    }
}

struct ResolveFixture {
    a_dir: PathBuf,
    b_dir: PathBuf,
    root_a: String,
    root_b: String,
    backend_a: LocalBackend,
    backend_b: LocalBackend,
    path_a: String,
    path_b: String,
    pair: String,
    conflict: Conflict,
}

impl Drop for ResolveFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.a_dir).ok();
        std::fs::remove_dir_all(&self.b_dir).ok();
        std::fs::remove_dir_all(versions_dir(&self.pair)).ok();
    }
}

#[test]
fn checked_resolution_copies_offered_winner_and_reports_phases() {
    let fixture = fixture("copy", Some(b"winner"), Some(b"loser"));
    let cancel = AtomicBool::new(false);
    let mut phases = Vec::new();

    let signatures = resolve_checked(
        &fixture.backend_a,
        &fixture.root_a,
        &fixture.backend_b,
        &fixture.root_b,
        &fixture.conflict,
        true,
        &fixture.pair,
        &cancel,
        |phase| phases.push(phase),
    )
    .unwrap();

    assert_eq!(std::fs::read(&fixture.path_a).unwrap(), b"winner");
    assert_eq!(std::fs::read(&fixture.path_b).unwrap(), b"winner");
    assert!(signatures.0.is_some() && signatures.1.is_some());
    assert_eq!(
        phases,
        vec![
            ResolvePhase::Preparing,
            ResolvePhase::BackingUp,
            ResolvePhase::Copying,
            ResolvePhase::ReadingSignatures,
        ]
    );
    assert!(tree_contains(&versions_dir(&fixture.pair), b"loser"));
}

#[test]
fn cancellation_before_resolution_never_mutates_either_side() {
    let fixture = fixture("cancel", Some(b"winner"), Some(b"loser"));
    let cancel = AtomicBool::new(true);
    let error = resolve_checked(
        &fixture.backend_a,
        &fixture.root_a,
        &fixture.backend_b,
        &fixture.root_b,
        &fixture.conflict,
        true,
        &fixture.pair,
        &cancel,
        |_| {},
    )
    .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(std::fs::read(&fixture.path_a).unwrap(), b"winner");
    assert_eq!(std::fs::read(&fixture.path_b).unwrap(), b"loser");
    assert!(!versions_dir(&fixture.pair).exists());
}

#[test]
fn cancellation_before_publish_leaves_destination_unchanged() {
    let fixture = fixture("cancel_publish", Some(b"winner"), Some(b"loser"));
    let cancel = AtomicBool::new(false);
    let error = resolve_checked(
        &fixture.backend_a,
        &fixture.root_a,
        &fixture.backend_b,
        &fixture.root_b,
        &fixture.conflict,
        true,
        &fixture.pair,
        &cancel,
        |phase| {
            if phase == ResolvePhase::Copying {
                cancel.store(true, Ordering::Release);
            }
        },
    )
    .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(std::fs::read(&fixture.path_a).unwrap(), b"winner");
    assert_eq!(std::fs::read(&fixture.path_b).unwrap(), b"loser");
}

#[test]
fn signature_drift_is_rejected_without_overwriting_destination() {
    let fixture = fixture("drift", Some(b"winner"), Some(b"loser"));
    std::fs::write(&fixture.path_a, b"changed after dialog").unwrap();
    let cancel = AtomicBool::new(false);

    let error = resolve_checked(
        &fixture.backend_a,
        &fixture.root_a,
        &fixture.backend_b,
        &fixture.root_b,
        &fixture.conflict,
        true,
        &fixture.pair,
        &cancel,
        |_| {},
    )
    .unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(std::fs::read(&fixture.path_b).unwrap(), b"loser");
}

#[test]
fn choosing_deleted_side_backs_up_then_deletes_other_side() {
    let fixture = fixture("delete", None, Some(b"loser"));
    let cancel = AtomicBool::new(false);
    let mut phases = Vec::new();

    let signatures = resolve_checked(
        &fixture.backend_a,
        &fixture.root_a,
        &fixture.backend_b,
        &fixture.root_b,
        &fixture.conflict,
        true,
        &fixture.pair,
        &cancel,
        |phase| phases.push(phase),
    )
    .unwrap();

    assert_eq!(signatures, (None, None));
    assert!(!Path::new(&fixture.path_a).exists());
    assert!(!Path::new(&fixture.path_b).exists());
    assert_eq!(
        phases,
        vec![
            ResolvePhase::Preparing,
            ResolvePhase::BackingUp,
            ResolvePhase::Deleting,
            ResolvePhase::ReadingSignatures,
        ]
    );
    assert!(tree_contains(&versions_dir(&fixture.pair), b"loser"));
}

fn tree_contains(root: &Path, expected: &[u8]) -> bool {
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if std::fs::read(path).ok().as_deref() == Some(expected) {
                return true;
            }
        }
    }
    false
}
