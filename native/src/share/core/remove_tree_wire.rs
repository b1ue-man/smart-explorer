//! Messages of a host-side folder delete (`remove_tree_v1`,
//! `FsResponse::RemoveTree`).
use serde::{Deserialize, Serialize};

/// The stream of one folder delete: counters every 250 ms while the host
/// checks (`applying` false) and removes (`applying` true) the tree, then
/// the closing totals. `planned` counts the folder and every entry below it
/// and is never below `removed`; a failure ends the stream with `Err`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "m", rename_all = "snake_case")]
pub(crate) enum FsRemoveTreeMessage {
    Progress {
        applying: bool,
        planned: u64,
        removed: u64,
    },
    Done {
        planned: u64,
        removed: u64,
    },
}

#[cfg(test)]
#[path = "remove_tree_wire_task_tests.rs"]
mod task_tests;
