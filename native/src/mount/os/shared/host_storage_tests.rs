//! Signals of vault drives in the shared host flow (DRV-M7, stage
//! drive-vault): the engine of a vault drive writes only sealed spool bytes
//! and sealed journal paths, a crash with a locked vault leaves
//! `RequiresUnlock` and the change is published after the unlock, and a
//! lock publishes, drops clean content and unmounts (or refuses).
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::super::HostControl;
use super::{prepare_for_lock, storage_for, LockOrder, SpoolKeyAnswer};
use crate::mount::optimization_fixture::{FixtureDirectory, OptimizationBackend};
use crate::mount::{
    MountEngine, MountId, MountMode, MountRecovery, MountRuntimeConfig, MountStatus, MountStorage,
    OpenDisposition, OpenFileOptions,
};
use crate::vault::SpoolKey;

const MARKER: &[u8] = b"KLARTEXT-MARKER-DRV-M7";
const KEY: [u8; 32] = [0x5a; 32];

type Report = (MountStatus, Option<MountRecovery>);

/// The daemon side of a test host: one key answer, recorded reports.
struct KeyControl {
    answer: Mutex<Option<SpoolKeyAnswer>>,
    reports: Mutex<Vec<Report>>,
}

impl KeyControl {
    fn new(answer: SpoolKeyAnswer) -> Self {
        Self {
            answer: Mutex::new(Some(answer)),
            reports: Mutex::new(Vec::new()),
        }
    }

    fn with_key(raw: &[u8; 32]) -> Self {
        Self::new(SpoolKeyAnswer::Key(SpoolKey::from_bytes(raw).unwrap()))
    }
}

impl HostControl for KeyControl {
    fn report(&self, status: MountStatus, recovery: Option<MountRecovery>) -> Result<(), String> {
        self.reports.lock().unwrap().push((status, recovery));
        Ok(())
    }

    fn wait_for_stop(&self, _timeout: Duration) -> bool {
        true
    }

    fn spool_key(&self) -> SpoolKeyAnswer {
        self.answer
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| SpoolKeyAnswer::Failed("no second key answer".into()))
    }
}

fn storage(root: &Path, id: &MountId, raw: &[u8; 32]) -> MountStorage {
    storage_for(&KeyControl::with_key(raw), true, root, id).unwrap()
}

fn open(
    root: &Path,
    backend: &Arc<OptimizationBackend>,
    storage: MountStorage,
    id: &MountId,
) -> std::io::Result<MountEngine> {
    let runtime = MountRuntimeConfig::new(id.clone(), MountMode::ReadWrite);
    MountEngine::open_host_cache_with(runtime, backend.clone(), root, storage)
}

fn writable() -> OpenFileOptions {
    OpenFileOptions {
        writable: true,
        disposition: OpenDisposition::OpenExisting,
    }
}

fn spool_files(root: &Path, id: &MountId) -> Vec<PathBuf> {
    fs::read_dir(root.join(id.as_str()).join("files"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "spool")
        })
        .collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn order(keep_unsaved: bool) -> LockOrder {
    LockOrder {
        deadline: Instant::now() + Duration::from_secs(60),
        keep_unsaved,
    }
}

#[test]
fn drive_task_vault_spool_uses_sealed_storage() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/note", b"old");
    let id = MountId::parse("vault-sealed-spool").unwrap();
    let engine = open(
        directory.path(),
        &backend,
        storage(directory.path(), &id, &KEY),
        &id,
    )
    .unwrap();
    let handle = engine.open_file("\\note", writable()).unwrap();
    assert_eq!(engine.write(handle, 0, MARKER).unwrap(), MARKER.len());
    let spools = spool_files(directory.path(), &id);
    assert_eq!(spools.len(), 1);
    let sealed = fs::read(&spools[0]).unwrap();
    assert!(!contains(&sealed, MARKER), "cleartext reached the spool");
    assert!(sealed.len() > MARKER.len());
    // The save publishes on flush; closing only releases the handle.
    engine.flush(handle).unwrap();
    engine.close(handle).unwrap();
    assert_eq!(backend.bytes("/note"), MARKER);
    drop(engine);

    // A drive without a vault keeps the plain spool of every release.
    let plain_id = MountId::parse("plain-spool").unwrap();
    let control = KeyControl::new(SpoolKeyAnswer::Failed("never asked".into()));
    let plain = storage_for(&control, false, directory.path(), &plain_id).unwrap();
    let engine = open(directory.path(), &backend, plain, &plain_id).unwrap();
    let handle = engine.open_file("\\note", writable()).unwrap();
    engine.write(handle, 0, b"plain bytes").unwrap();
    let spools = spool_files(directory.path(), &plain_id);
    assert_eq!(fs::read(&spools[0]).unwrap(), b"plain bytes");
    assert!(
        control.answer.lock().unwrap().is_some(),
        "a plain drive asked for a key"
    );
    engine.close(handle).unwrap();
}

#[test]
fn drive_task_vault_journal_path_fields_sealed() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/GEHEIM-PLAN.txt", b"old");
    let id = MountId::parse("vault-journal").unwrap();
    let engine = open(
        directory.path(),
        &backend,
        storage(directory.path(), &id, &KEY),
        &id,
    )
    .unwrap();
    let handle = engine.open_file("\\GEHEIM-PLAN.txt", writable()).unwrap();
    engine.write(handle, 0, MARKER).unwrap();
    let journal = fs::read(directory.path().join(id.as_str()).join("journal.jsonl")).unwrap();
    assert!(!journal.is_empty());
    assert!(
        !contains(&journal, b"GEHEIM-PLAN"),
        "a path field stayed plain"
    );
    assert!(!contains(&journal, MARKER));
    // The process ends without closing: the sealed record is the recovery.
    drop(engine);

    let reopened = open(
        directory.path(),
        &backend,
        storage(directory.path(), &id, &KEY),
        &id,
    )
    .unwrap();
    let dirty = reopened.dirty_entries().unwrap();
    assert_eq!(dirty.len(), 1);
    assert_eq!(dirty[0].0, "/GEHEIM-PLAN.txt");
    drop(reopened);
    // Another spool key opens neither journal nor spool.
    let other = storage(directory.path(), &id, &[0x11; 32]);
    assert!(open(directory.path(), &backend, other, &id).is_err());
}

#[test]
fn drive_task_vault_requires_unlock_after_crash() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/note", b"old");
    let id = MountId::parse("vault-crash").unwrap();
    let engine = open(
        directory.path(),
        &backend,
        storage(directory.path(), &id, &KEY),
        &id,
    )
    .unwrap();
    let handle = engine.open_file("\\note", writable()).unwrap();
    engine.write(handle, 0, MARKER).unwrap();
    drop(engine);

    // The vault is locked when the host starts again: nothing mounts, the
    // sealed change is counted without the key.
    let control = KeyControl::new(SpoolKeyAnswer::Locked(
        "Tresor Privat ist gesperrt: se vault unlock 0123".into(),
    ));
    let refused = storage_for(&control, true, directory.path(), &id)
        .err()
        .unwrap();
    assert!(refused.starts_with("vault_locked: "), "{refused}");
    let reports = control.reports.lock().unwrap().clone();
    match reports.last() {
        Some((MountStatus::Failed { detail }, recovery)) => {
            assert!(detail.starts_with("vault_locked: "), "{detail}");
            assert_eq!(
                *recovery,
                Some(MountRecovery::RequiresUnlock { pending: 1 })
            );
        }
        other => panic!("unexpected report {other:?}"),
    }
    let wire = serde_json::to_value(MountRecovery::RequiresUnlock { pending: 1 }).unwrap();
    assert_eq!(wire, serde_json::json!({"requires_unlock": {"pending": 1}}));
    assert_eq!(MountRecovery::awaiting_unlock(0), MountRecovery::Clean);
    assert_eq!(backend.bytes("/note"), b"old");

    // After the unlock the host gets its key and publishes the change.
    let engine = open(
        directory.path(),
        &backend,
        storage(directory.path(), &id, &KEY),
        &id,
    )
    .unwrap();
    engine.retry_pending_changes().unwrap();
    assert_eq!(backend.bytes("/note"), MARKER);
    assert!(engine.dirty_entries().unwrap().is_empty());
}

#[test]
fn drive_task_vault_prepare_lock_flushes_evicts_unmounts() {
    let directory = FixtureDirectory::new();
    let backend = OptimizationBackend::new();
    backend.put("/note", b"old");
    let id = MountId::parse("vault-lock").unwrap();
    let engine = open(
        directory.path(),
        &backend,
        storage(directory.path(), &id, &KEY),
        &id,
    )
    .unwrap();
    let handle = engine.open_file("\\note", writable()).unwrap();
    engine.write(handle, 0, MARKER).unwrap();

    // Publishing fails: a lock someone asked for is refused, the drive
    // stays mounted; a forced lock (or „verschlüsselt aufbewahren“)
    // unmounts with the change sealed.
    backend.fail_upload(true);
    let (readiness, unmount) = prepare_for_lock(&engine, &order(false));
    assert!(readiness.pending > 0);
    assert!(!unmount);
    let (_, unmount) = prepare_for_lock(&engine, &order(true));
    assert!(unmount);
    assert_eq!(backend.bytes("/note"), b"old");

    // Publishing works: the change reaches the remote, clean cached
    // content goes, the drive unmounts.
    backend.fail_upload(false);
    engine.close(handle).unwrap();
    let (readiness, unmount) = prepare_for_lock(&engine, &order(false));
    assert_eq!(readiness.pending, 0);
    assert!(unmount);
    assert_eq!(backend.bytes("/note"), MARKER);
    assert!(
        spool_files(directory.path(), &id).is_empty(),
        "clean cached content stayed"
    );
}
