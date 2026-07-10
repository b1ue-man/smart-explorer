use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::vfs::{Backend, ChangeKind, VfsChange, VfsMeta};

use super::core::sig_eq;
use super::paths::join;
use super::snapshot::{hash_mode, md5_hex_to_u64, walk_files, WalkFilter};
use super::state_store::{ItemRecord, PairRecord, Side, SyncStateStore};
use super::types::{BisyncOptions, Sig, Tree};

const MAX_CHANGE_NODES: usize = 1_000_000;
const MAX_CHANGE_TEXT_BYTES: usize = 128 * 1024 * 1024;
const MAX_CHANGE_DEPTH: usize = 512;

#[derive(Clone, Copy)]
pub(super) struct CollectionLimits {
    max_nodes: usize,
    max_text_bytes: usize,
    max_depth: usize,
}

impl CollectionLimits {
    const STANDARD: Self = Self {
        max_nodes: MAX_CHANGE_NODES,
        max_text_bytes: MAX_CHANGE_TEXT_BYTES,
        max_depth: MAX_CHANGE_DEPTH,
    };

    #[cfg(test)]
    pub(super) const fn new(max_nodes: usize, max_text_bytes: usize, max_depth: usize) -> Self {
        Self {
            max_nodes,
            max_text_bytes,
            max_depth,
        }
    }
}

struct CollectionBudget {
    limits: CollectionLimits,
    nodes: usize,
    text_bytes: usize,
}

impl CollectionBudget {
    fn new(limits: CollectionLimits) -> Self {
        Self {
            limits,
            nodes: 0,
            text_bytes: 0,
        }
    }

    fn record_node(&mut self, text_bytes: usize) -> bool {
        self.nodes = self.nodes.saturating_add(1);
        self.record_text(text_bytes) && self.nodes <= self.limits.max_nodes
    }

    fn record_text(&mut self, text_bytes: usize) -> bool {
        self.text_bytes = self.text_bytes.saturating_add(text_bytes);
        self.text_bytes <= self.limits.max_text_bytes
    }
}

#[derive(Debug)]
pub(super) enum ChangeCollection {
    Ready {
        changes: Vec<ResolvedChange>,
        new_cursor: Option<String>,
    },
    Rebuild,
    Canceled,
}

#[derive(Clone, Debug)]
pub(super) struct ResolvedChange {
    pub(super) rel: String,
    pub(super) old_rel: Option<String>,
    pub(super) kind: ChangeKind,
    pub(super) id: Option<String>,
    pub(super) parent_id: Option<String>,
    pub(super) name: Option<String>,
    pub(super) source_sig: Option<Sig>,
    pub(super) managed: bool,
    pub(super) old_managed: bool,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn changes_from_backend(
    store: &SyncStateStore,
    rec: &PairRecord,
    source: &dyn Backend,
    root: &str,
    side: Side,
    source_items: &BTreeMap<String, ItemRecord>,
    filter: &WalkFilter,
    cancel: &AtomicBool,
) -> ChangeCollection {
    changes_from_backend_with_limits(
        store,
        rec,
        source,
        root,
        side,
        source_items,
        filter,
        cancel,
        CollectionLimits::STANDARD,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn changes_from_backend_with_limits(
    store: &SyncStateStore,
    rec: &PairRecord,
    source: &dyn Backend,
    root: &str,
    side: Side,
    source_items: &BTreeMap<String, ItemRecord>,
    filter: &WalkFilter,
    cancel: &AtomicBool,
    limits: CollectionLimits,
) -> ChangeCollection {
    let Some(cursor) = rec.source_cursor.as_deref() else {
        return ChangeCollection::Rebuild;
    };
    if cancel.load(Ordering::Relaxed) {
        return ChangeCollection::Canceled;
    }
    let batch = match source.changes_since(root, cursor) {
        Ok(batch) => batch,
        Err(_) if cancel.load(Ordering::Relaxed) => return ChangeCollection::Canceled,
        Err(_) => return ChangeCollection::Rebuild,
    };
    if cancel.load(Ordering::Relaxed) {
        return ChangeCollection::Canceled;
    }
    if batch.reset {
        return ChangeCollection::Rebuild;
    }
    let mut budget = CollectionBudget::new(limits);
    if !budget.record_text(batch.new_cursor.as_deref().map_or(0, str::len)) {
        return ChangeCollection::Rebuild;
    }
    let mut changes = Vec::new();
    let mut changed_paths = BTreeSet::new();
    for raw in batch.changes {
        if cancel.load(Ordering::Relaxed) {
            return ChangeCollection::Canceled;
        }
        if !budget.record_node(raw_text_bytes(&raw)) {
            return ChangeCollection::Rebuild;
        }
        let (mut change, supplied_meta) = match resolve_change(store, rec, side, raw) {
            Ok(Some(change)) => change,
            Ok(None) | Err(_) => return ChangeCollection::Rebuild,
        };
        if !normalize_change_paths(&mut change, limits.max_depth)
            || !budget.record_text(change.rel.len() + change.old_rel.as_deref().map_or(0, str::len))
            || !changed_paths.insert(change.rel.clone())
        {
            return ChangeCollection::Rebuild;
        }
        match &change.kind {
            ChangeKind::Upsert => {
                let metadata = match supplied_meta {
                    Some(metadata) => metadata,
                    None => match source.stat(&join(root, &change.rel)) {
                        Ok(metadata) => metadata,
                        Err(_) if cancel.load(Ordering::Relaxed) => {
                            return ChangeCollection::Canceled
                        }
                        Err(_) => return ChangeCollection::Rebuild,
                    },
                };
                let Some(sig) = sig_from_meta(&metadata) else {
                    return ChangeCollection::Rebuild;
                };
                change.source_sig = Some(sig);
                change.managed = metadata_in_scope(&change.rel, &metadata, filter);
                change.old_managed = change.managed
                    && change
                        .old_rel
                        .as_deref()
                        .is_some_and(|old| item_in_scope(old, source_items.get(old), filter));
            }
            ChangeKind::Remove => {
                change.managed = item_in_scope(&change.rel, source_items.get(&change.rel), filter);
                change.old_managed = false;
                change.source_sig = None;
            }
        }
        changes.push(change);
    }
    ChangeCollection::Ready {
        changes,
        new_cursor: batch.new_cursor,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn changes_from_source_walk(
    source: &dyn Backend,
    root: &str,
    target: &dyn Backend,
    opts: BisyncOptions,
    filter: &WalkFilter,
    source_items: &BTreeMap<String, ItemRecord>,
    cancel: &AtomicBool,
) -> ChangeCollection {
    let prev_tree: Tree = source_items
        .iter()
        .filter_map(|(rel, item)| {
            (!item.deleted)
                .then_some(item.sig)
                .flatten()
                .map(|sig| (rel.clone(), sig))
        })
        .collect();
    let mode = hash_mode(source, target, opts.compare);
    let current = match walk_files(source, root, cancel, filter, mode, Some(&prev_tree)) {
        Ok(current) => current,
        Err(error) if error.kind() == io::ErrorKind::Interrupted => {
            return ChangeCollection::Canceled
        }
        Err(_) => return ChangeCollection::Rebuild,
    };
    let mut budget = CollectionBudget::new(CollectionLimits::STANDARD);
    let mut changes = Vec::new();
    let rels = current
        .keys()
        .chain(prev_tree.keys().filter(|rel| !current.contains_key(*rel)));
    for rel in rels {
        if cancel.load(Ordering::Relaxed) {
            return ChangeCollection::Canceled;
        }
        if !budget.record_node(rel.len()) || !valid_rel(rel, MAX_CHANGE_DEPTH) {
            return ChangeCollection::Rebuild;
        }
        let now = current.get(rel).copied();
        let previous = prev_tree.get(rel).copied();
        if sig_eq(now, previous, &opts) {
            continue;
        }
        let kind = now.map_or(ChangeKind::Remove, |_| ChangeKind::Upsert);
        let managed = match now {
            Some(_) => true,
            None if !item_in_scope(rel, source_items.get(rel), filter) => false,
            None => match source.stat(&join(root, rel)) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => true,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                    return ChangeCollection::Canceled
                }
                Err(_) => return ChangeCollection::Rebuild,
                Ok(metadata) if !metadata_in_scope(rel, &metadata, filter) => false,
                Ok(_) => return ChangeCollection::Rebuild,
            },
        };
        let id = match &kind {
            ChangeKind::Upsert => match source.item_id(&join(root, rel)) {
                Ok(id) => id,
                Err(_) if cancel.load(Ordering::Relaxed) => return ChangeCollection::Canceled,
                Err(_) => return ChangeCollection::Rebuild,
            },
            ChangeKind::Remove => source_items.get(rel).and_then(|item| item.id.clone()),
        };
        let name = rel.rsplit('/').next().map(str::to_owned);
        if !budget
            .record_text(id.as_deref().map_or(0, str::len) + name.as_deref().map_or(0, str::len))
        {
            return ChangeCollection::Rebuild;
        }
        changes.push(ResolvedChange {
            rel: rel.clone(),
            old_rel: None,
            kind,
            id,
            parent_id: None,
            name,
            source_sig: now,
            managed,
            old_managed: false,
        });
    }
    ChangeCollection::Ready {
        changes,
        new_cursor: None,
    }
}

fn resolve_change(
    store: &SyncStateStore,
    rec: &PairRecord,
    side: Side,
    raw: VfsChange,
) -> rusqlite::Result<Option<(ResolvedChange, Option<VfsMeta>)>> {
    let VfsChange {
        kind,
        rel,
        id,
        parent_id,
        name,
        meta,
    } = raw;
    let id_rel = match id.as_deref() {
        Some(id) => store.rel_for_id(&rec.pair, side, id)?,
        None => None,
    };
    let parent_rel = rel_from_parent(store, rec, side, parent_id.as_deref(), name.as_deref())?;
    let parent_addressed = parent_id.is_some() && name.is_some();
    let Some(rel) = rel
        .or(parent_rel)
        .or_else(|| (!parent_addressed).then(|| id_rel.clone()).flatten())
    else {
        return Ok(None);
    };
    let old_rel = id_rel.filter(|old| old != &rel);
    Ok(Some((
        ResolvedChange {
            rel,
            old_rel,
            kind,
            id,
            parent_id,
            name,
            source_sig: meta.as_ref().and_then(sig_from_meta),
            managed: false,
            old_managed: false,
        },
        meta,
    )))
}

fn rel_from_parent(
    store: &SyncStateStore,
    rec: &PairRecord,
    side: Side,
    parent_id: Option<&str>,
    name: Option<&str>,
) -> rusqlite::Result<Option<String>> {
    let (Some(parent_id), Some(name)) = (parent_id, name) else {
        return Ok(None);
    };
    let root_id = if side == Side::A {
        rec.root_a_id.as_deref()
    } else {
        rec.root_b_id.as_deref()
    };
    let parent = if root_id == Some(parent_id) {
        Some(String::new())
    } else {
        store.rel_for_id(&rec.pair, side, parent_id)?
    };
    Ok(parent.map(|parent| {
        if parent.is_empty() {
            name.to_owned()
        } else {
            format!("{parent}/{name}")
        }
    }))
}

fn normalize_change_paths(change: &mut ResolvedChange, max_depth: usize) -> bool {
    let Ok(rel) = crate::agent_proto::ValidatedRelativePath::parse(&change.rel) else {
        return false;
    };
    if rel.as_str().split('/').count() > max_depth {
        return false;
    }
    change.rel = rel.as_str().to_owned();
    if let Some(old) = change.old_rel.as_mut() {
        let Ok(normalized) = crate::agent_proto::ValidatedRelativePath::parse(old) else {
            return false;
        };
        if normalized.as_str().split('/').count() > max_depth {
            return false;
        }
        *old = normalized.as_str().to_owned();
    }
    true
}

fn valid_rel(rel: &str, max_depth: usize) -> bool {
    crate::agent_proto::ValidatedRelativePath::parse(rel)
        .is_ok_and(|path| path.as_str().split('/').count() <= max_depth)
}

fn metadata_in_scope(rel: &str, metadata: &VfsMeta, filter: &WalkFilter) -> bool {
    !metadata.is_dir
        && !metadata.is_symlink
        && path_in_scope(rel, metadata.hidden, filter)
        && filter.size_age_ok(metadata.size, metadata.mtime_ms)
}

fn item_in_scope(rel: &str, item: Option<&ItemRecord>, filter: &WalkFilter) -> bool {
    item.filter(|item| !item.deleted)
        .and_then(|item| item.sig)
        .is_some_and(|sig| {
            path_in_scope(rel, false, filter) && filter.size_age_ok(sig.size, sig.mtime_ms)
        })
}

fn path_in_scope(rel: &str, hidden: bool, filter: &WalkFilter) -> bool {
    !filter.ignore.is_match(rel)
        && (filter.include_hidden
            || (!hidden
                && !rel
                    .rsplit('/')
                    .next()
                    .is_some_and(|name| name.starts_with('.'))))
}

fn sig_from_meta(metadata: &VfsMeta) -> Option<Sig> {
    if metadata.is_dir || metadata.is_symlink {
        return None;
    }
    Some(Sig {
        size: metadata.size,
        mtime_ms: metadata.mtime_ms,
        hash: metadata
            .content_md5
            .as_deref()
            .map(md5_hex_to_u64)
            .unwrap_or(0),
    })
}

fn raw_text_bytes(raw: &VfsChange) -> usize {
    let mut bytes = raw
        .rel
        .as_deref()
        .map_or(0, str::len)
        .saturating_add(raw.id.as_deref().map_or(0, str::len))
        .saturating_add(raw.parent_id.as_deref().map_or(0, str::len))
        .saturating_add(raw.name.as_deref().map_or(0, str::len));
    if let Some(metadata) = raw.meta.as_ref() {
        bytes = bytes
            .saturating_add(metadata.name.len())
            .saturating_add(metadata.id.as_deref().map_or(0, str::len))
            .saturating_add(metadata.content_md5.as_deref().map_or(0, str::len));
    }
    bytes
}

#[cfg(test)]
#[path = "tests/incremental_safety.rs"]
mod tests;
