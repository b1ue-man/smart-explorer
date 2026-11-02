//! The fake remote's keyed walk (`hash_walk_keyed`, a host with
//! `hash_walk_keys_v1`): the folder keys of the tree under a root, formed by
//! the walk the agent runs next to its data, and counted.
use super::Fake;
use crate::vfs::{HashWalkRequest, KeyedWalk, KeyedWalkItem, VfsResult};
use std::sync::atomic::{AtomicBool, Ordering};

impl Fake {
    /// `KeyedWalk::KeysOnly` with `meta`, when `Faults::keyed` says so;
    /// `Ok(false)` (unsupported) for anything else.
    pub(super) fn keyed_walk(
        &self,
        root: &str,
        request: HashWalkRequest,
        walk: KeyedWalk<'_>,
        tx: crossbeam_channel::Sender<KeyedWalkItem>,
        cancel: &AtomicBool,
    ) -> VfsResult<bool> {
        if !self.faults.keyed || !request.meta || !matches!(walk, KeyedWalk::KeysOnly) {
            return Ok(false);
        }
        self.counters.key_walks.fetch_add(1, Ordering::SeqCst);
        let keys = crate::agent_proto::LocalKeyed::keys_of(
            root,
            crate::agent_proto::digest::NONE,
            cancel,
        )?;
        for (rel, key, tally) in keys {
            let _ = tx.send(KeyedWalkItem::Key {
                rel,
                key,
                same: false,
                tally: Some(tally),
            });
        }
        Ok(true)
    }
}
