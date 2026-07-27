use super::engine::Entry;
use crate::vfs::VfsMeta;
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct OpenHandle {
    pub(super) kind: OpenHandleKind,
    pub(super) writable: bool,
}

#[derive(Clone)]
pub(super) enum OpenHandleKind {
    Materialized(Arc<Entry>),
    /// An open-existing handle whose contents were not fetched yet. The first
    /// data operation upgrades it in place to a materialized entry; pure
    /// metadata traffic (Explorer's bulk) never transfers the file at all.
    Metadata {
        callback_path: String,
        meta: VfsMeta,
    },
}

impl OpenHandle {
    pub(super) fn references(&self, entry: &Arc<Entry>) -> bool {
        matches!(&self.kind, OpenHandleKind::Materialized(opened) if Arc::ptr_eq(opened, entry))
    }
}
