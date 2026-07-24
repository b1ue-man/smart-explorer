use super::engine::Entry;
use crate::vfs::VfsMeta;
use std::{io, sync::Arc};

#[derive(Clone)]
pub(super) struct OpenHandle {
    pub(super) kind: OpenHandleKind,
    pub(super) writable: bool,
}

#[derive(Clone)]
pub(super) enum OpenHandleKind {
    Materialized(Arc<Entry>),
    Metadata(VfsMeta),
}

impl OpenHandle {
    pub(super) fn materialized_entry(&self) -> io::Result<Arc<Entry>> {
        match &self.kind {
            OpenHandleKind::Materialized(entry) => Ok(Arc::clone(entry)),
            OpenHandleKind::Metadata(_) => Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "file handle was opened for metadata only",
            )),
        }
    }

    pub(super) fn references(&self, entry: &Arc<Entry>) -> bool {
        matches!(&self.kind, OpenHandleKind::Materialized(opened) if Arc::ptr_eq(opened, entry))
    }
}
