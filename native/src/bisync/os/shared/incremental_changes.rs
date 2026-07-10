use std::collections::{BTreeMap, BTreeSet};

use crate::vfs::{Backend, ChangeKind, VfsMeta};

use super::core::sig_eq;
use super::incremental_collect::ResolvedChange;
use super::paths::join;
use super::snapshot::md5_hex_to_u64;
use super::state_store::{ItemRecord, Side};
use super::types::{Action, Baseline, BisyncOptions, Sig, Tree};

#[derive(Debug, Default)]
pub(super) struct IncrementalActionPlan {
    pub(super) upserts: Vec<Action>,
    pub(super) deletes: Vec<Action>,
}

pub(super) fn action_plan_for(
    source_side: Side,
    changes: &[ResolvedChange],
) -> IncrementalActionPlan {
    let final_paths: BTreeSet<&str> = changes
        .iter()
        .filter(|change| change.managed && change.kind == ChangeKind::Upsert)
        .map(|change| change.rel.as_str())
        .collect();
    let mut plan = IncrementalActionPlan::default();
    let mut delete_paths = BTreeSet::new();
    for change in changes.iter().filter(|change| change.managed) {
        match &change.kind {
            ChangeKind::Upsert => {
                plan.upserts.push(match source_side {
                    Side::A => Action::CopyAtoB(change.rel.clone()),
                    Side::B => Action::CopyBtoA(change.rel.clone()),
                });
                if change.old_managed {
                    if let Some(old) = change.old_rel.as_deref() {
                        delete_paths.insert(old);
                    }
                }
            }
            ChangeKind::Remove => {
                delete_paths.insert(change.rel.as_str());
            }
        }
    }
    plan.deletes = delete_paths
        .into_iter()
        .filter(|rel| !final_paths.contains(rel))
        .map(|rel| match source_side {
            Side::A => Action::DeleteB(rel.to_owned()),
            Side::B => Action::DeleteA(rel.to_owned()),
        })
        .collect();
    plan
}

pub(super) fn apply_trees(
    source_side: Side,
    source_items: &BTreeMap<String, ItemRecord>,
    target_items: &BTreeMap<String, ItemRecord>,
    changes: &[ResolvedChange],
) -> Option<(Tree, Tree)> {
    let mut source = item_tree(source_items);
    let target = item_tree(target_items);
    for change in changes.iter().filter(|change| change.managed) {
        if change.old_managed {
            if let Some(old_rel) = change.old_rel.as_deref() {
                source.remove(old_rel);
            }
        }
        if change.kind == ChangeKind::Remove {
            source.remove(&change.rel);
        }
    }
    // Insert all final paths only after every old path was retired. In a rename
    // swap/cycle an old path is another change's final path, so interleaving
    // remove+insert would erase a signature that was just inserted.
    for change in changes
        .iter()
        .filter(|change| change.managed && change.kind == ChangeKind::Upsert)
    {
        source.insert(change.rel.clone(), change.source_sig?);
    }
    Some(match source_side {
        Side::A => (source, target),
        Side::B => (target, source),
    })
}

fn item_tree(items: &BTreeMap<String, ItemRecord>) -> Tree {
    items
        .iter()
        .filter_map(|(rel, item)| {
            (!item.deleted && !item.is_dir)
                .then_some(item.sig)
                .flatten()
                .map(|signature| (rel.clone(), signature))
        })
        .collect()
}

pub(super) fn target_touched_drifted(
    target: &dyn Backend,
    root: &str,
    target_items: &BTreeMap<String, ItemRecord>,
    changes: &[ResolvedChange],
    opts: BisyncOptions,
) -> bool {
    for ch in changes.iter().filter(|change| change.managed) {
        if target_rel_drifted(target, root, target_items, &ch.rel, opts) {
            return true;
        }
        if ch.old_managed
            && ch
                .old_rel
                .as_deref()
                .is_some_and(|old| target_rel_drifted(target, root, target_items, old, opts))
        {
            return true;
        }
    }
    false
}

fn target_rel_drifted(
    target: &dyn Backend,
    root: &str,
    target_items: &BTreeMap<String, ItemRecord>,
    rel: &str,
    opts: BisyncOptions,
) -> bool {
    let expected = target_items
        .get(rel)
        .and_then(|i| (!i.deleted).then_some(i.sig).flatten());
    let actual = match target.stat(&join(root, rel)) {
        Ok(metadata) => sig_from_meta(&metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return true,
    };
    !sig_eq(actual, expected, &opts)
}

pub(super) fn delete_guard_trips(
    plan: &IncrementalActionPlan,
    target_items: &BTreeMap<String, ItemRecord>,
    opts: BisyncOptions,
) -> bool {
    let deletes = plan
        .upserts
        .iter()
        .chain(plan.deletes.iter())
        .filter(|action| {
            matches!(
                action,
                Action::DeleteA(_)
                    | Action::DeleteB(_)
                    | Action::FinalizeMoveAtoB(_)
                    | Action::FinalizeMoveBtoA(_)
            )
        })
        .count() as u64;
    if deletes == 0 {
        return false;
    }
    let total = target_items.values().filter(|i| !i.deleted).count() as u64;
    (opts.max_delete > 0 && deletes > opts.max_delete)
        || (opts.max_delete_pct > 0 && deletes > total * opts.max_delete_pct as u64 / 100)
}

pub(super) fn source_item_after(side: Side, ch: &ResolvedChange) -> ItemRecord {
    ItemRecord {
        side,
        rel: ch.rel.clone(),
        id: ch.id.clone(),
        parent_id: ch.parent_id.clone(),
        name: ch
            .name
            .clone()
            .or_else(|| ch.rel.rsplit('/').next().map(|s| s.to_string())),
        sig: ch.source_sig,
        is_dir: false,
        deleted: ch.kind == ChangeKind::Remove,
    }
}

pub(super) fn target_item_after(
    target: &dyn Backend,
    root: &str,
    side: Side,
    ch: &ResolvedChange,
) -> std::io::Result<ItemRecord> {
    let path = join(root, &ch.rel);
    let meta = target.stat(&path)?;
    let sig = sig_from_meta(&meta).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "incremental target is not a regular file",
        )
    })?;
    let id = target.item_id(&path)?;
    Ok(ItemRecord {
        side,
        rel: ch.rel.clone(),
        id: id.or(meta.id),
        parent_id: parent_id_for(target, root, &ch.rel),
        name: ch
            .name
            .clone()
            .or_else(|| ch.rel.rsplit('/').next().map(|s| s.to_string())),
        sig: Some(sig),
        is_dir: false,
        deleted: false,
    })
}

pub(super) fn deleted_item(side: Side, rel: &str, prev: Option<&ItemRecord>) -> ItemRecord {
    ItemRecord {
        side,
        rel: rel.to_string(),
        id: prev.and_then(|p| p.id.clone()),
        parent_id: prev.and_then(|p| p.parent_id.clone()),
        name: prev
            .and_then(|p| p.name.clone())
            .or_else(|| rel.rsplit('/').next().map(|s| s.to_string())),
        sig: None,
        is_dir: false,
        deleted: true,
    }
}

fn sig_from_meta(m: &VfsMeta) -> Option<Sig> {
    if m.is_dir || m.is_symlink {
        return None;
    }
    Some(Sig {
        size: m.size,
        mtime_ms: m.mtime_ms,
        hash: m.content_md5.as_deref().map(md5_hex_to_u64).unwrap_or(0),
    })
}

pub(super) fn collect_ids(
    be: &dyn Backend,
    root: &str,
    baseline: &Baseline,
    side: Side,
) -> BTreeMap<String, (Option<String>, Option<String>)> {
    baseline
        .iter()
        .filter_map(|(rel, (a, b))| {
            let present = (if side == Side::A { a } else { b }).is_some();
            present.then(|| {
                (
                    rel.clone(),
                    (
                        be.item_id(&join(root, rel)).ok().flatten(),
                        parent_id_for(be, root, rel),
                    ),
                )
            })
        })
        .collect()
}

fn parent_id_for(be: &dyn Backend, root: &str, rel: &str) -> Option<String> {
    let parent_path = rel
        .rsplit_once('/')
        .map_or_else(|| root.to_string(), |(p, _)| join(root, p));
    be.item_id(&parent_path).ok().flatten()
}
