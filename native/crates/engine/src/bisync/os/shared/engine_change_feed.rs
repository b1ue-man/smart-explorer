//! Root ancestry for account-wide feeds; a cache is trusted only after a complete walk.
use super::incremental::SyncEndpoints;
use super::incremental_collect::ResolvedChange;
use super::state_store::{
    upsert_item_tx, write_pair, ItemRecord, PairRecord, Side, SyncStateStore,
};
use super::types::{Baseline, BisyncOptions};
use crate::vfs::{Backend, ChangeKind, VfsChange, VfsMeta};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::atomic::AtomicBool;

pub(super) enum FeedResolution {
    Rooted(ResolvedChange, Option<VfsMeta>),
    Outside,
}

pub(super) struct FeedIndex<'a> {
    root: Option<&'a str>,
    by_id: BTreeMap<&'a str, &'a ItemRecord>,
    batch: BTreeMap<&'a str, &'a VfsChange>,
}

fn rebuild() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "change ancestry or duplicate identity requires a full scan",
    )
}

impl<'a> FeedIndex<'a> {
    pub(super) fn new(
        record: &'a PairRecord,
        side: Side,
        items: &'a BTreeMap<String, ItemRecord>,
        changes: &'a [VfsChange],
    ) -> io::Result<Self> {
        let root = if side == Side::A {
            record.root_a_id.as_deref()
        } else {
            record.root_b_id.as_deref()
        };
        let mut by_id = BTreeMap::new();
        for item in items.values().filter(|item| !item.deleted) {
            if let Some(id) = item.id.as_deref() {
                if Some(id) == root || by_id.insert(id, item).is_some() {
                    return Err(rebuild());
                }
            }
        }
        let mut batch = BTreeMap::new();
        for raw in changes {
            if let Some(id) = raw.id.as_deref() {
                // Repeated account events can represent several transitions.
                // A full scan observes their terminal state without guessing.
                if batch.insert(id, raw).is_some() {
                    return Err(rebuild());
                }
            }
        }
        Ok(Self { root, by_id, batch })
    }

    /// Some(path) proves descent from the selected root; None proves an
    /// account object with no parent outside it. Missing evidence is an error.
    fn parent(&self, id: &str, seen: &mut BTreeSet<String>) -> io::Result<Option<String>> {
        if seen.len() >= 512 || !seen.insert(id.to_string()) {
            return Err(rebuild());
        }
        if self.root == Some(id) {
            if self.batch.contains_key(id) {
                return Err(rebuild());
            }
            return Ok(Some(String::new()));
        }
        if let Some(item) = self.by_id.get(id) {
            if !item.is_dir || self.batch.contains_key(id) {
                return Err(rebuild());
            }
            let parent = item.parent_id.as_deref().ok_or_else(rebuild)?;
            let prefix = self.parent(parent, seen)?.ok_or_else(rebuild)?;
            let name = item.name.as_deref().ok_or_else(rebuild)?;
            let expected = child(&prefix, name)?;
            if expected != item.rel {
                return Err(rebuild());
            }
            return Ok(Some(expected));
        }
        let raw = self.batch.get(id).ok_or_else(rebuild)?;
        let meta = raw.meta.as_ref().ok_or_else(rebuild)?;
        if raw.kind != ChangeKind::Upsert || !meta.is_dir || meta.is_symlink || meta.special {
            return Err(rebuild());
        }
        match raw.parent_id.as_deref() {
            None => Ok(None),
            Some(parent) => match self.parent(parent, seen)? {
                None => Ok(None),
                // A new/moved folder in this root requires a complete
                // observation of its children and duplicate groups.
                Some(_) => Err(rebuild()),
            },
        }
    }

    pub(super) fn resolve(&self, raw: &VfsChange) -> io::Result<FeedResolution> {
        let previous = raw.id.as_deref().and_then(|id| self.by_id.get(id).copied());
        if raw.id.as_deref().is_some_and(|id| self.root == Some(id)) {
            return Err(rebuild());
        }
        if previous.is_some_and(|item| item.is_dir) {
            return Err(rebuild());
        }
        let rel = if let Some(rel) = &raw.rel {
            // Root-relative feeds retain their existing explicit contract.
            super::sync_relative_path::SyncRelativePath::parse(rel)?;
            rel.clone()
        } else {
            let id = raw
                .id
                .as_deref()
                .filter(|id| !id.is_empty())
                .ok_or_else(rebuild)?;
            if self.root.is_none() {
                return Err(rebuild());
            }
            if raw.kind == ChangeKind::Remove {
                let Some(item) = previous else {
                    // The index contains every managed ID, including folders.
                    // An absent removed ID cannot authorize a root deletion.
                    return Ok(FeedResolution::Outside);
                };
                item.rel.clone()
            } else {
                let Some(parent) = raw.parent_id.as_deref() else {
                    if previous.is_some() {
                        return Err(rebuild());
                    }
                    return Ok(FeedResolution::Outside);
                };
                let Some(prefix) = self.parent(parent, &mut BTreeSet::new())? else {
                    if previous.is_some() {
                        return Err(rebuild());
                    }
                    return Ok(FeedResolution::Outside);
                };
                let meta = raw.meta.as_ref().ok_or_else(rebuild)?;
                if meta.is_dir || meta.is_symlink || meta.special || meta.id.as_deref() != Some(id)
                {
                    return Err(rebuild());
                }
                let name = raw.name.as_deref().ok_or_else(rebuild)?;
                if meta.name != name {
                    return Err(rebuild());
                }
                child(&prefix, name)?
            }
        };
        let change = ResolvedChange {
            old_rel: previous
                .map(|item| item.rel.clone())
                .filter(|old| old != &rel),
            rel,
            kind: raw.kind.clone(),
            id: raw.id.clone(),
            parent_id: raw.parent_id.clone(),
            name: raw.name.clone(),
            source_sig: None,
            managed: false,
            old_managed: false,
        };
        Ok(FeedResolution::Rooted(change, raw.meta.clone()))
    }
}

fn child(parent: &str, name: &str) -> io::Result<String> {
    super::sync_relative_path::validate_component(name)?;
    let rel = if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    };
    super::sync_relative_path::SyncRelativePath::parse(&rel)?;
    Ok(rel)
}

/// Include directories and complete observed name groups in the same
/// transaction as the cursor. File-only historic caches are never upgraded
/// to account ancestry merely by receiving a new cursor.
pub(super) fn bootstrap(
    store: &mut SyncStateStore,
    record: &PairRecord,
    endpoints: SyncEndpoints<'_>,
    baseline: &Baseline,
    opts: BisyncOptions,
    filter: &super::WalkFilter<'_>,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let mut rows = Vec::new();
    let keys = super::orchestration_plan::keys(endpoints);
    let limits = super::SyncLimits::for_memory(crate::transfer::physical_memory());
    let mut text = 0u64;
    for (side, backend, root, other) in [
        (Side::A, endpoints.a, endpoints.root_a, endpoints.b),
        (Side::B, endpoints.b, endpoints.root_b, endpoints.a),
    ] {
        let previous = super::snapshot::prev_side(baseline, side == Side::A);
        let snapshot = super::snapshot::walk_snapshot_with_options(
            backend,
            root,
            cancel,
            filter,
            super::snapshot::hash_mode(backend, other, opts.compare),
            Some(&previous),
            true,
            keys.fold_case,
            opts,
        )?;
        if !snapshot.omissions.is_empty()
            || !snapshot.filtered.is_empty()
            || !snapshot.duplicates.is_empty()
            || snapshot.tree.len() != previous.len()
        {
            return Err(rebuild());
        }
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        for (rel, is_dir) in snapshot
            .tree
            .keys()
            .map(|rel| (rel, false))
            .chain(snapshot.dirs.iter().map(|rel| (rel, true)))
        {
            super::transfer_stream::check(cancel)?;
            if !names.insert(keys.key(rel).into_owned()) {
                return Err(rebuild());
            }
            let path = crate::vfs::sync_path(backend, root, rel)?;
            let meta = backend.stat(&path)?;
            if meta.is_symlink || meta.special || meta.is_dir != is_dir {
                return Err(rebuild());
            }
            let signature = if is_dir {
                None
            } else {
                let expected = previous.get(rel).ok_or_else(rebuild)?;
                if meta.size != expected.size
                    || meta.mtime_ms != expected.mtime_ms
                    || meta.content_md5.as_deref().is_some_and(|hash| {
                        expected.hash != 0 && super::snapshot::md5_hex_to_u64(hash) != expected.hash
                    })
                {
                    return Err(rebuild());
                }
                Some(*expected)
            };
            let id = backend.item_id(&path)?.or(meta.id);
            if id.as_ref().is_some_and(|id| !ids.insert(id.clone())) {
                return Err(rebuild());
            }
            let parent = rel.rsplit_once('/').map_or("", |(parent, _)| parent);
            let parent_id = backend.item_id(&crate::vfs::sync_path(backend, root, parent)?)?;
            if backend.supports_changes() && (id.is_none() || parent_id.is_none()) {
                return Err(rebuild());
            }
            text = text
                .saturating_add(rel.len() as u64)
                .saturating_add(id.as_ref().map_or(0, |id| id.len() as u64))
                .saturating_add(parent_id.as_ref().map_or(0, |id| id.len() as u64));
            rows.push(ItemRecord {
                side,
                rel: rel.clone(),
                id,
                parent_id,
                name: rel.rsplit('/').next().map(str::to_string),
                sig: signature,
                is_dir,
                deleted: false,
            });
            if rows.len() as u64 > limits.state_entries || text > limits.state_text_bytes {
                return Err(rebuild());
            }
        }
    }
    let tx = store.conn.transaction().map_err(io::Error::other)?;
    tx.execute("DELETE FROM items WHERE pair = ?1", [&record.pair])
        .map_err(io::Error::other)?;
    for row in &rows {
        upsert_item_tx(&tx, &record.pair, row).map_err(io::Error::other)?;
    }
    write_pair(&tx, record).map_err(io::Error::other)?;
    tx.commit().map_err(io::Error::other)
}
