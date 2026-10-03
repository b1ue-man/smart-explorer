//! SSH analysis keeps the older server-side walk, with this receiver's
//! budget registered before the first reply. An oversized old tree is
//! drained without allocation and uses tolerant, bounded listings instead.
use std::sync::atomic::Ordering;

use super::backend::AgentBackend;
use crate::analytics::{Progress, ScanOutcome, ScanPhase};
use crate::agent_proto::TreeDecodeBudget;
use crate::vfs::VfsResult;

impl AgentBackend {
    pub(super) fn scan_storage_budgeted(&self, root: &str, progress: &Progress)
        -> VfsResult<Option<ScanOutcome>> {
        let host = self.inner.scan_storage(root, progress)?;
        if host.is_some() || self.features().service { return Ok(host); }
        let files = progress.files.load(Ordering::Relaxed);
        let bytes = progress.bytes.load(Ordering::Relaxed);
        progress.set_phase(ScanPhase::Legacy, root);
        let offered = progress.node_budget();
        progress.set_node_budget(offered);
        let budget = TreeDecodeBudget::new(offered, crate::transfer::memory_budget());
        let on_progress = |done, total| {
            progress.files.store(files.saturating_add(done), Ordering::Relaxed);
            progress.bytes.store(bytes.saturating_add(total), Ordering::Relaxed);
            progress.check_cancel().is_ok()
        };
        match self.walk_tree_with_budget(root, &on_progress, Some(budget)) {
            Ok(Some(tree)) if progress.check_cancel().is_ok() =>
                Ok(Some(crate::analytics::finish_legacy_tree(tree, progress, files, bytes))),
            Err(error) if error.kind() == std::io::ErrorKind::OutOfMemory => {
                progress.check_cancel()?;
                progress.files.store(files, Ordering::Relaxed);
                progress.bytes.store(bytes, Ordering::Relaxed);
                let mut outcome = crate::analytics::scan_backend(self, root, progress);
                outcome.notes.push("Der ältere Baum-Walk überschreitet das Empfängerbudget; die Detailansicht wurde über begrenzte Ordnerlisten aufgebaut.".into());
                Ok(Some(outcome))
            }
            Ok(None) => Ok(None),
            Ok(Some(_)) => Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "agent analysis canceled")),
            Err(error) => Err(error),
        }
    }
}
