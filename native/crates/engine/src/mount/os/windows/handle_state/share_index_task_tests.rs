use super::super::{HandleRecord, NodeHandle, State};
use super::super::super::handle_access::share_allows;
use std::collections::HashSet;

const READ: u32 = 1;
const WRITE: u32 = 2;
const DELETE: u32 = 0x0001_0000;
const ALL_SHARES: u32 = 7;
const ACCESS: [u32; 15] = [
    0, READ, WRITE, 4, 0x20, DELETE, READ | WRITE, READ | DELETE,
    0x80, 0x0010_0000, 0x0200_0000, 0x1000_0000, 0x2000_0000,
    0x4000_0000, 0x8000_0000,
];

fn record(path: &str, access: u32, share: u32) -> HandleRecord {
    HandleRecord {
        node: Some(NodeHandle::File(crate::mount::HandleId(1))),
        path: path.into(), is_directory: false, desired_access: access,
        share_access: share, share_active: true, namespace_attached: true,
        delete_requested: false, delete_committed: false,
    }
}

fn assert_pairwise(state: &State, path: &str) {
    for access in ACCESS {
        for share in 0..=ALL_SHARES {
            let pairwise = state.handles.values().filter(|record| {
                record.namespace_attached && record.share_active && record.path == path
            }).all(|record| {
                share_allows(record.share_access, access)
                    && share_allows(share, record.desired_access)
            });
            assert_eq!(state.shares.allows(path, access, share), pairwise,
                "path={path}, access={access:#x}, share={share:#x}");
        }
    }
    let attached = state.handles.values()
        .any(|record| record.namespace_attached && record.path == path);
    assert_eq!(state.shares.has_attached(path), attached);
    let excluded = std::iter::once(None).chain(state.handles.values().map(Some));
    for except in excluded {
        let allowed = state.handles.values().all(|record| {
            !record.namespace_attached || !record.share_active || record.path != path
                || record.share_access & 4 != 0
                || except.is_some_and(|except| std::ptr::eq(except, record))
        });
        assert_eq!(state.shares.delete_allowed_except(path, except), allowed);
    }
}

#[test]
fn mount_vault_task_share_aggregates_match_pairwise_masks() {
    for access in ACCESS {
        for share in 0..=ALL_SHARES {
            let mut state = State::default();
            state.insert_handle(1, record(r"\same", access, share)).unwrap();
            // Independent and duplicate contributions exercise every aggregate
            // category, not just the one-record equivalence.
            state.insert_handle(2, record(r"\same", READ | DELETE, ALL_SHARES)).unwrap();
            state.insert_handle(3, record(r"\other", WRITE, 0)).unwrap();
            assert_pairwise(&state, r"\same");
            state.cleanup_handle(1).unwrap();
            state.cleanup_handle(1).unwrap();
            assert_pairwise(&state, r"\same");
            state.remove_handle(1).unwrap();
            state.remove_handle(2).unwrap();
            assert_pairwise(&state, r"\same");
            assert_pairwise(&state, r"\other");
            state.remove_handle(3).unwrap();
            assert!(state.shares.paths.is_empty());
        }
    }
}

#[test]
fn mount_vault_task_share_lifecycle_detach_rename_and_same_key() {
    let mut state = State::default();
    state.insert_handle(1, record(r"\source", READ | DELETE, ALL_SHARES)).unwrap();
    state.insert_handle(2, record(r"\source\child", WRITE, 0)).unwrap();
    state.insert_handle(3, record(r"\destination", READ, 0)).unwrap();
    state.insert_handle(4, record(r"\other", READ, ALL_SHARES)).unwrap();
    state.cleanup_handle(3).unwrap();
    assert!(state.shares.has_attached(r"\destination"));
    assert_pairwise(&state, r"\destination");

    state.rename_attached(r"\source", r"\source", true).unwrap();
    assert!(state.handles[&1].namespace_attached);
    assert_pairwise(&state, r"\source");
    state.rename_attached(r"\source", r"\destination", true).unwrap();
    assert!(!state.handles[&3].namespace_attached);
    assert_eq!(state.handles[&1].path, r"\destination");
    assert_eq!(state.handles[&2].path, r"\destination\child");
    for path in [r"\source", r"\source\child", r"\destination", r"\destination\child", r"\other"] {
        assert_pairwise(&state, path);
    }
    // Closing the old detached object must not subtract the new occupant.
    state.remove_handle(3).unwrap();
    assert_pairwise(&state, r"\destination");
    state.handles.get_mut(&1).unwrap().delete_requested = true;
    state.complete_delete(r"\destination", &HashSet::from([1]));
    state.complete_delete(r"\destination", &HashSet::from([1]));
    assert!(state.handles[&1].delete_committed);
    assert!(!state.handles[&1].delete_requested);
    state.insert_handle(5, record(r"\destination", WRITE, 0)).unwrap();
    state.remove_handle(1).unwrap();
    assert_pairwise(&state, r"\destination");
    for key in [2, 4, 5] { state.remove_handle(key).unwrap(); }
    assert!(state.handles.is_empty());
    assert!(state.shares.paths.is_empty());
}
