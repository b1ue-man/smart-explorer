//! Remote-to-remote copies of the older entry point run as engine jobs:
//! server-side inside one account, streamed between connections, bridged
//! through a local temporary file only where one connection cannot read and
//! write at once.
use super::engine::{run_legacy, Side};
use super::job::JobItems;
use super::types::TransferMsg;
use crate::types::FilterDef;
use std::sync::atomic::AtomicBool;

/// `_same_server` is kept for callers; the engine decides from the
/// endpoints' namespace identity itself.
// This worker entry point keeps source, destination, progress reporting, and
// cancellation inputs explicit because they cross the background-task boundary.
#[allow(clippy::too_many_arguments)]
pub fn copy_remote_paths_progress(
    src: &dyn crate::vfs::Backend,
    paths: &[String],
    tgt: &dyn crate::vfs::Backend,
    dest_root: &str,
    _same_server: bool,
    filter: Option<(FilterDef, String)>,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    cancel: &AtomicBool,
) {
    run_legacy(
        Side::Remote(src),
        Side::Remote(tgt),
        dest_root,
        JobItems::Roots {
            paths: paths.to_vec(),
            base: None,
        },
        filter,
        tx,
        cancel,
    );
}
