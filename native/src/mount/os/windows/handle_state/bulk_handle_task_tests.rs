use super::{HandleTable, NodeHandle};
use super::super::handle_access::{requests_delete, requests_read, requests_write};
use std::io;

const READ: u32 = 1;
const WRITE: u32 = 2;
const DELETE: u32 = 0x0001_0000;
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const ALL_SHARES: u32 = 7;

fn commit_file(table: &HandleTable, path: &str, access: u32, share: u32) -> u64 {
    let reservation = table.reserve(path, false, access, share).unwrap();
    reservation.bind(NodeHandle::File(crate::mount::HandleId(reservation.key()))).unwrap();
    reservation.commit()
}

#[test]
fn mount_vault_task_handle_reservation_abort_cleanup_and_grants() {
    let table = HandleTable::new(false, false);
    let unbound = table.reserve(r"\Note", false, READ, 1).unwrap();
    assert!(table.snapshot(unbound.key()).is_err());
    assert!(matches!(table.reserve(r"\note", false, WRITE, ALL_SHARES),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock));
    drop(unbound);
    assert!(table.lock_state().unwrap().handles.is_empty());

    let read = commit_file(&table, r"\note", READ, 1);
    let maximum = table.reserve(r"\note", false, MAXIMUM_ALLOWED, ALL_SHARES).unwrap();
    assert!(requests_read(maximum.granted_access()));
    assert!(!requests_write(maximum.granted_access()));
    assert!(!requests_delete(maximum.granted_access()));
    drop(maximum);
    table.cleanup(read).unwrap();
    table.cleanup(read).unwrap();
    let full = table.reserve(r"\note", false, MAXIMUM_ALLOWED, ALL_SHARES).unwrap();
    assert!(requests_read(full.granted_access()));
    assert!(requests_write(full.granted_access()));
    assert!(requests_delete(full.granted_access()));
    drop(full);
    assert!(table.lock_state().unwrap().shares.has_attached(&table.path_key(r"\note")));
    table.take(read).unwrap();
    assert!(!table.lock_state().unwrap().shares.has_attached(&table.path_key(r"\note")));

    let read_only = HandleTable::new(true, true);
    let grant = read_only.reserve(r"\read-only", false, MAXIMUM_ALLOWED, ALL_SHARES).unwrap();
    assert!(requests_read(grant.granted_access()));
    assert!(!requests_write(grant.granted_access()));
    assert!(!requests_delete(grant.granted_access()));
    drop(grant);

    // Directory admission is exempt; its active delete sharing still matters.
    let directory = table.reserve(r"\folder", true, DELETE, 0).unwrap();
    directory.bind(NodeHandle::Directory).unwrap();
    let directory = directory.commit();
    let second = table.reserve(r"\folder", true, READ, 0).unwrap();
    second.bind(NodeHandle::Directory).unwrap();
    let second = second.commit();
    assert!(table.reserve_rename(directory, r"\folder", r"\moved", false).is_err());
    table.cleanup(second).unwrap();
    table.reserve_rename(directory, r"\folder", r"\moved", false).unwrap().commit().unwrap();
    assert_eq!(table.snapshot(directory).unwrap().path, table.path_key(r"\moved"));
    table.take(directory).unwrap();
    table.take(second).unwrap();
    assert!(table.lock_state().unwrap().handles.is_empty());
}

#[test]
fn mount_vault_task_handle_same_identity_rename_preserves_sharing() {
    let table = HandleTable::new(false, false);
    let handle = commit_file(&table, r"\Mixed", DELETE, 0);
    let rename = table.reserve_rename(handle, r"\Mixed", r"\mixed", true).unwrap();
    assert!(!rename.destination_is_open());
    rename.commit().unwrap();
    assert_eq!(table.snapshot(handle).unwrap().path, table.path_key(r"\Mixed"));
    assert!(matches!(table.reserve(r"\mixed", false, READ, ALL_SHARES),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock));
    table.cleanup(handle).unwrap();
    let reader = commit_file(&table, r"\mixed", READ, ALL_SHARES);
    table.take(handle).unwrap();
    table.take(reader).unwrap();
    assert!(table.lock_state().unwrap().handles.is_empty());
}

#[test]
fn mount_vault_task_ten_thousand_same_and_unrelated_handles_drain() {
    for same_path in [true, false] {
        let table = HandleTable::new(true, false);
        let paths = (0..10_000).map(|index| {
            if same_path { r"\same".to_string() } else { format!(r"\item-{index}") }
        }).collect::<Vec<_>>();
        let handles = paths.iter().map(|path| commit_file(&table, path, READ, ALL_SHARES))
            .collect::<Vec<_>>();
        assert_eq!(table.lock_state().unwrap().handles.len(), paths.len());
        for &key in handles.iter().step_by(2) {
            table.cleanup(key).unwrap();
            table.cleanup(key).unwrap();
        }
        // Close in a different order to exercise sparse live handle storage.
        for &key in handles.iter().rev() { table.take(key).unwrap(); }
        let state = table.lock_state().unwrap();
        assert!(state.handles.is_empty());
        for path in paths {
            assert!(!state.shares.has_attached(&path));
            assert!(state.shares.allows(&path, READ | WRITE | DELETE, ALL_SHARES));
        }
    }
}
