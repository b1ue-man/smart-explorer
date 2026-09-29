//! Filtered local clipboard pairs retain their validated relative hierarchy.
use super::engine::{run_legacy, Side};
use super::job::{JobItems, PairItem};
use super::types::TransferMsg;
use std::sync::atomic::AtomicBool;

/// Uploads explicit (absolute source, relative destination) pairs through
/// the streaming engine; the set is validated as a whole first, and each
/// top-level name is reserved once (numbered when taken).
pub fn upload_pairs_progress(
    backend: &dyn crate::vfs::Backend,
    pairs: &[(String, String)],
    destination: &str,
    tx: &crossbeam_channel::Sender<TransferMsg>,
    cancel: &AtomicBool,
) {
    let items = JobItems::Pairs(
        pairs
            .iter()
            .map(|(source, rel)| PairItem::new(source.clone(), rel.clone()))
            .collect(),
    );
    run_legacy(
        Side::Local,
        Side::Remote(backend),
        destination,
        items,
        None,
        tx,
        cancel,
    );
}
