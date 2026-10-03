//! Remote-suite signals for capture/restart/restore; never run on this workstation.
use std::{ffi::OsStr, io::{self, Write}, path::PathBuf};
use crate::{local_access::DirectoryHandle, vfs::RecycleOutcome};
use super::{catalog, platform, record::{EntryState, Record, RestoreOutcome}, restore, store::Store};

struct Fixture {
    parent: DirectoryHandle,
    file: std::fs::File,
    record: Record,
    root: PathBuf,
    // Pins/files must drop before TempDir attempts its ordinary cleanup.
    temp: tempfile::TempDir,
}
fn fixture() -> io::Result<Fixture> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("source");
    std::fs::create_dir(&root)?;
    std::fs::write(root.join("copy"), b"same")?;
    let root = std::fs::canonicalize(root)?;
    let parent = DirectoryHandle::open_root(&root)?;
    let file = parent.open_regular_child(OsStr::new("copy"))?;
    let record = platform::test_intent(&root, &file, 4)?;
    Ok(Fixture { parent, file, record, root, temp })
}
fn store(f: &Fixture) -> io::Result<Store> { Store::at(&f.temp.path().join("records")) }
fn state(store: &Store, id: &str) -> io::Result<EntryState> {
    Ok(catalog::list_in(store, None)?.entries.into_iter().find(|entry| entry.id == id)
        .ok_or_else(|| io::Error::other("Missing intent"))?.state)
}

#[test]
fn review_task_host_trash_intent_precedes_capture_and_survives_restart() -> io::Result<()> {
    let f = fixture()?; let store = store(&f)?;
    store.persist(&f.record)?;
    assert_eq!(state(&store, &f.record.id)?, EntryState::OriginalPresent);
    let slot = DirectoryHandle::checked_quarantine_slot(OsStr::new(&f.record.held))?;
    let captured = f.parent.quarantine_regular_child_in(OsStr::new("copy"), &f.file, &slot)?;
    assert_eq!(captured.retained_location().file_name(), Some(slot.name()));
    drop(captured); drop(store);
    let reopened = Store::at(&f.temp.path().join("records"))?;
    assert_eq!(state(&reopened, &f.record.id)?, EntryState::Held);
    assert_eq!(restore::restore_in(&reopened, &reopened.load(&f.record.id)?)?, RestoreOutcome::Restored);
    assert_eq!(std::fs::read(f.root.join("copy"))?, b"same");
    Ok(())
}

#[test]
fn review_task_host_trash_restore_restart_hop_has_durable_mapping() -> io::Result<()> {
    let f = fixture()?; let store = store(&f)?;
    assert_eq!(platform::capture(&store, &f.record, &f.parent, &f.file)?, RecycleOutcome::Recycled);
    let held = f.parent.open_regular_child(OsStr::new(&f.record.held))?;
    let next = DirectoryHandle::checked_quarantine_slot(OsStr::new(&f.record.restore_held))?;
    let captured = f.parent.quarantine_regular_child_in(OsStr::new(&f.record.held), &held, &next)?;
    drop(captured); drop(held); drop(store);
    let reopened = Store::at(&f.temp.path().join("records"))?;
    assert_eq!(state(&reopened, &f.record.id)?, EntryState::RestorePending);
    assert_eq!(restore::restore_in(&reopened, &reopened.load(&f.record.id)?)?, RestoreOutcome::Restored);
    assert_eq!(std::fs::read(f.root.join("copy"))?, b"same");
    Ok(())
}

#[test]
fn review_task_host_trash_restore_preserves_collision_and_is_retryable() -> io::Result<()> {
    let f = fixture()?; let store = store(&f)?;
    platform::capture(&store, &f.record, &f.parent, &f.file)?;
    std::fs::write(f.root.join("copy"), b"replacement")?;
    assert!(restore::restore_in(&store, &f.record).is_err());
    assert_eq!(std::fs::read(f.root.join("copy"))?, b"replacement");
    assert_eq!(state(&store, &f.record.id)?, EntryState::Held);
    std::fs::remove_file(f.root.join("copy"))?;
    assert_eq!(restore::restore_in(&store, &f.record)?, RestoreOutcome::Restored);
    assert_eq!(std::fs::read(f.root.join("copy"))?, b"same");
    Ok(())
}

#[test]
fn review_task_host_trash_changed_payload_is_never_restored() -> io::Result<()> {
    let f = fixture()?; let store = store(&f)?;
    platform::capture(&store, &f.record, &f.parent, &f.file)?;
    std::fs::write(f.root.join(&f.record.held), b"DIFF")?;
    assert!(restore::restore_in(&store, &f.record).is_err());
    assert_eq!(std::fs::read(f.root.join(&f.record.held))?, b"DIFF");
    assert!(!f.root.join("copy").exists());
    Ok(())
}

#[test]
fn review_task_host_trash_failed_intent_and_expected_hash_leave_source_untouched() -> io::Result<()> {
    let f = fixture()?; let store = store(&f)?;
    drop(store.area.directory.create_file_new(OsStr::new(&format!("{}.json", f.record.id)))?);
    assert!(platform::capture(&store, &f.record, &f.parent, &f.file).is_err());
    assert_eq!(std::fs::read(f.root.join("copy"))?, b"same");
    assert!(!f.root.join(&f.record.held).exists());
    let expected = crate::vfs::RecycleExpectation { size:4, sha256:Some("0".repeat(64)) };
    assert_eq!(crate::analytics::recycle_local(&f.root, &f.root.join("copy"), &expected)?, RecycleOutcome::Changed);
    assert_eq!(std::fs::read(f.root.join("copy"))?, b"same");
    Ok(())
}

#[test]
fn review_task_host_trash_replaced_root_is_not_a_restore_target() -> io::Result<()> {
    let f = fixture()?; let store = store(&f)?;
    platform::capture(&store, &f.record, &f.parent, &f.file)?;
    let Fixture { parent, file, record, root, temp } = f;
    drop(file); drop(parent);
    let old = root.with_file_name("old-source");
    std::fs::rename(&root, &old)?;
    std::fs::create_dir(&root)?;
    std::fs::write(root.join("copy"), b"foreign")?;
    assert!(restore::restore_in(&store, &record).is_err());
    assert_eq!(std::fs::read(root.join("copy"))?, b"foreign");
    assert_eq!(std::fs::read(old.join(&record.held))?, b"same");
    drop(store); drop(temp);
    Ok(())
}

#[test]
fn review_task_host_trash_link_source_and_invalid_slots_are_refused() -> io::Result<()> {
    let f = fixture()?;
    for invalid in ["", "..", ".held.se-recycle-deadbeef", ".held.se-recycle-0123456789ABCDEf", "sub/.held.se-recycle-0123456789abcdef"] {
        assert!(DirectoryHandle::checked_quarantine_slot(OsStr::new(invalid)).is_err());
    }
    let outside = f.temp.path().join("outside");
    std::fs::write(&outside, b"same")?;
    platform::test_symlink_file(&outside, &f.root.join("linked"))?;
    let expected = crate::vfs::RecycleExpectation { size:4, sha256:Some(f.record.sha256.clone()) };
    assert!(crate::analytics::recycle_local(&f.root, &f.root.join("linked"), &expected).is_err());
    assert_eq!(std::fs::read(outside)?, b"same");
    Ok(())
}

#[test]
fn review_task_host_trash_private_records_and_partial_intents_keep_other_entries() -> io::Result<()> {
    let f = fixture()?; let store = store(&f)?;
    platform::capture(&store, &f.record, &f.parent, &f.file)?;
    let broken = "00000000000000000000000000000001";
    let mut file = store.area.directory.create_file_new(OsStr::new(&format!("{broken}.json")))?;
    file.write_all(b"{")?; file.sync_all()?; drop(file);
    assert_eq!(state(&store, broken)?, EntryState::Problem);
    assert_eq!(state(&store, &f.record.id)?, EntryState::Held);
    let alias = f.temp.path().join("record-alias");
    std::fs::hard_link(store.area.path.join(format!("{}.json", f.record.id)), &alias)?;
    assert!(store.load(&f.record.id).is_err(), "hardlinked recovery metadata must not be read");
    Ok(())
}

#[test]
fn review_task_host_trash_catalog_pages_without_dropping_intents() -> io::Result<()> {
    let f = fixture()?; let store = store(&f)?;
    for id in 1..=130 {
        let name = format!("{id:032x}.json");
        let mut file = store.area.directory.create_file_new(OsStr::new(&name))?;
        file.write_all(b"{")?;
    }
    let first = catalog::list_in(&store, None)?;
    let second = catalog::list_in(&store, first.next.as_deref())?;
    let third = catalog::list_in(&store, second.next.as_deref())?;
    assert_eq!(first.entries.len(), 64); assert_eq!(second.entries.len(), 64); assert_eq!(third.entries.len(), 2);
    assert!(third.next.is_none());
    let mut ids = first.entries.into_iter().chain(second.entries).chain(third.entries).map(|entry| entry.id).collect::<Vec<_>>();
    ids.sort(); ids.dedup(); assert_eq!(ids.len(), 130);
    Ok(())
}

#[test]
fn review_task_host_trash_file_reservation_serializes_other_owners() -> io::Result<()> {
    let f = fixture()?; let locked = store(&f)?;
    assert!(Store::at(&locked.area.path).is_err());
    let root = locked.area.path.clone(); drop(locked);
    assert!(Store::at(&root).is_ok());
    Ok(())
}
