//! Folders a caller settles without a listing (`WalkOptions::settled`): the
//! hook's type, the depth a walk lists, and the walk's use of the hook.
use super::{Walk, MAX_WALK_DEPTH};
use crate::vfs::KeyTally;

/// Whether the folder at `rel` (its path below the target folder) in the
/// given depth below its root is settled as a whole: what its subtree holds.
pub type Settled<'a> = &'a (dyn Fn(&str, usize) -> Option<KeyTally> + Sync);

/// The deepest folder level a walk lists (`MAX_WALK_DEPTH`): a settled
/// subtree must not reach beyond it, or the listing would have reported it.
pub const DEPTH_LIMIT: usize = MAX_WALK_DEPTH;

impl Walk<'_> {
    /// The settled subtree of the folder at `rel` in `depth`, when the caller
    /// settles it. Never for a flattened or a filtered walk: its files are
    /// not the files of the folder.
    pub(super) fn settled(&self, rel: &str, depth: usize) -> Option<KeyTally> {
        if self.options.flatten || self.options.filter.is_some() {
            return None;
        }
        self.options.settled.and_then(|settled| settled(rel, depth))
    }
}
