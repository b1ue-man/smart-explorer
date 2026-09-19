//! Scan-time retention: the contract a recursive walker uses to decide which
//! entries are worth emitting while a filter is active, plus the lineage
//! bookkeeping that emits the directories a retained entry needs in the tree.
//!
//! Without retention every visited entry is emitted (the historical
//! behavior). With retention only entries the caller wants to keep are
//! emitted, together with each not-yet-emitted ancestor directory, so the
//! tree view can still place them. Progress counters keep counting every
//! visited entry; only emitted entries claim the bounded scan budget.
use crate::types::FileEntry;
use std::sync::{Arc, Mutex};

/// Decides what a walker keeps. Implementations must be cheap: they run once
/// per visited entry on the scan threads.
pub trait ScanRetention: Send + Sync {
    /// Keep this entry (file or directory) as a result.
    fn retain(&self, entry: &FileEntry) -> bool;
    /// Descend into this directory. Returning `false` prunes the whole
    /// subtree, which is only correct when no descendant could be retained.
    fn descend(&self, directory: &FileEntry) -> bool;
}

pub type RetentionHandle = Arc<dyn ScanRetention>;

/// A directory whose entry has not necessarily been emitted yet, linked to its
/// own pending ancestors. The first retained descendant emits the chain.
pub struct Lineage {
    entry: FileEntry,
    emitted: Mutex<bool>,
    parent: Option<Arc<Lineage>>,
}

impl Lineage {
    /// A directory entry that is *not* emitted yet.
    pub fn pending(entry: FileEntry, parent: Option<Arc<Lineage>>) -> Arc<Self> {
        Arc::new(Self {
            entry,
            emitted: Mutex::new(false),
            parent,
        })
    }

    /// The directory entry this lineage stands for.
    pub fn entry(&self) -> &FileEntry {
        &self.entry
    }

    /// Emit every not-yet-emitted ancestor, nearest first, through `emit`.
    /// Each directory is emitted exactly once even when several scan threads
    /// find their first match below it concurrently. Stops early (returning
    /// `false`) when `emit` refuses an entry, e.g. because the budget is gone.
    pub fn emit_pending(
        this: &Option<Arc<Lineage>>,
        mut emit: impl FnMut(&FileEntry) -> bool,
    ) -> bool {
        let mut current = this.as_ref();
        while let Some(lineage) = current {
            let mut emitted = lineage.emitted.lock().unwrap_or_else(|error| error.into_inner());
            if *emitted {
                break;
            }
            if !emit(&lineage.entry) {
                return false;
            }
            *emitted = true;
            drop(emitted);
            current = lineage.parent.as_ref();
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(path: &str) -> FileEntry {
        FileEntry {
            path: Arc::from(path),
            parent: Arc::from(""),
            name: Arc::from(path.rsplit('/').next().unwrap_or(path)),
            ext: Arc::from(""),
            size: 0,
            mtime_ms: 0,
            btime_ms: 0,
            is_dir: true,
            is_symlink: false,
            hidden: false,
            system: false,
            depth: 1,
            id: None,
        }
    }

    #[test]
    fn recursive_filter_task_lineage_emits_each_pending_ancestor_once() {
        let root_child = Lineage::pending(directory("/root/a"), None);
        let deeper = Lineage::pending(directory("/root/a/b"), Some(root_child.clone()));
        let sibling = Lineage::pending(directory("/root/a/c"), Some(root_child.clone()));

        let mut emitted = Vec::new();
        assert!(Lineage::emit_pending(&Some(deeper.clone()), |entry| {
            emitted.push(entry.path.to_string());
            true
        }));
        assert_eq!(emitted, ["/root/a/b", "/root/a"]);

        let mut again = Vec::new();
        assert!(Lineage::emit_pending(&Some(sibling), |entry| {
            again.push(entry.path.to_string());
            true
        }));
        assert_eq!(
            again,
            ["/root/a/c"],
            "the shared ancestor is not emitted twice"
        );

        let mut third = Vec::new();
        assert!(Lineage::emit_pending(&Some(deeper), |entry| {
            third.push(entry.path.to_string());
            true
        }));
        assert!(third.is_empty());
        assert!(Lineage::emit_pending(&None, |_| false));
    }

    #[test]
    fn recursive_filter_task_lineage_stops_when_the_sink_refuses() {
        let outer = Lineage::pending(directory("/root/a"), None);
        let inner = Lineage::pending(directory("/root/a/b"), Some(outer.clone()));
        let mut seen = 0;
        assert!(!Lineage::emit_pending(&Some(inner), |_| {
            seen += 1;
            false
        }));
        assert_eq!(seen, 1);
        assert!(!*outer.emitted.lock().unwrap());
    }
}
