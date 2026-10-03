//! Direct inotify coverage of a previously authorized DirectoryHandle.
//! No path reopen, recursive walk or child-name authorization occurs here.
//! Polling remains required for descendants and mount/server-side changes.

use std::os::unix::fs::MetadataExt;

use super::super::service::emit;
use super::super::types::{Coverage, UnavailableReason, WatchEvent, WatchId};
use super::{reason_of, AddError, Tree, FIRST_BACKOFF};

impl Tree {
    pub(super) fn arm_confined(
        &mut self,
        id: WatchId,
        anchor: crate::local_access::DirectoryHandle,
    ) {
        let metadata = match anchor.metadata() {
            Ok(metadata) if metadata.is_dir() && metadata.nlink() != 0 => metadata,
            Ok(_) => return self.fail(id, UnavailableReason::RootMissing),
            Err(error) => return self.fail(id, reason_of(&error)),
        };
        let Some(path) = anchor.watch_path() else {
            return self.fail(id, UnavailableReason::Unsupported);
        };
        if let Some(root) = self.roots.get_mut(&id) {
            root.dev = metadata.dev();
        }
        // The RootSpec and this local clone own the descriptor while the
        // kernel resolves /proc/self/fd/N/.; no literal child path is opened.
        match self.add_dir(id, &path, "") {
            Ok(()) => {
                if let Some(root) = self.roots.get_mut(&id) {
                    root.retry_at = None;
                    root.backoff = FIRST_BACKOFF;
                    root.reported = None;
                }
                emit(id, vec![WatchEvent::Ready(Coverage::LocalOnly)]);
            }
            Err(AddError::Limit) => self.fail(id, UnavailableReason::WatchLimit),
            Err(AddError::Other(reason)) => self.fail(id, reason),
        }
    }
}
