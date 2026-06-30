use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::vfs::{Backend, ChangeKind, VfsChange, VfsMeta};

use super::apply::apply_with_results;
use super::core::sig_eq;
use super::orchestration::Outcome;
use super::paths::join;
use super::persistence::{baseline_path, pair_id, save_baseline, versions_dir};
use super::snapshot::{hash_mode, md5_hex_to_u64, walk_files, WalkFilter};
use super::state_store::{ItemRecord, PairRecord, Side, SyncStateStore};
use super::types::{Action, Baseline, BisyncOptions, Conflict, DeletePolicy, Direction, Sig, Tree};

struct ResolvedChange {
    rel: String,
    old_rel: Option<String>,
    kind: ChangeKind,
    id: Option<String>,
    parent_id: Option<String>,
    name: Option<String>,
    source_sig: Option<Sig>,
}

pub(super) fn mirror_source<'a>(
    a: &'a dyn Backend,
    root_a: &'a str,
    b: &'a dyn Backend,
    root_b: &'a str,
    opts: BisyncOptions,
) -> Option<(&'a dyn Backend, &'a str, Side)> {
    if opts.delete != DeletePolicy::Mirror {
        return None;
    }
    match opts.direction {
        Direction::AtoB => Some((a, root_a, Side::A)),
        Direction::BtoA => Some((b, root_b, Side::B)),
        Direction::Both => None,
    }
}

pub(super) fn try_incremental_mirror(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    cancel: &AtomicBool,
    filter: &WalkFilter,
    store_path: Option<&Path>,
) -> Option<Outcome> {
    if opts.dry_run {
        return None;
    }
    let (source, source_root, source_side) = mirror_source(a, root_a, b, root_b, opts)?;
    let (target, target_root, target_side) = if source_side == Side::A {
        (b, root_b, Side::B)
    } else {
        (a, root_a, Side::A)
    };
    let pair = pair_id(root_a, root_b);
    let mut store = open_store(store_path).ok()?;
    let rec = store.load_pair(&pair).ok().flatten()?;
    if !record_matches(&rec, root_a, root_b, source_side) {
        return None;
    }
    if !root_id_matches(a, root_a, rec.root_a_id.as_deref())
        || !root_id_matches(b, root_b, rec.root_b_id.as_deref())
    {
        return None;
    }
    let source_items = store.load_side(&pair, source_side).ok()?;
    let target_items = store.load_side(&pair, target_side).ok()?;
    let (changes, new_cursor) = if source.supports_changes() {
        changes_from_backend(&store, &rec, source, source_root, source_side)?
    } else if source.is_local() {
        changes_from_source_walk(source, source_root, target, opts, filter, &source_items)?
    } else {
        return None;
    };
    if changes.is_empty() {
        if let Some(c) = new_cursor.as_deref() {
            let _ = store.update_cursor(&pair, Some(c));
        }
        return Some(Outcome {
            baseline: store.load_baseline(&pair).unwrap_or_default(),
            ..Default::default()
        });
    }
    if target_touched_drifted(target, target_root, &target_items, &changes, opts) {
        return None;
    }
    let actions = actions_for(source_side, &changes);
    if delete_guard_trips(&actions, &target_items, opts) {
        return Some(Outcome {
            errors: vec![(
                "abgebrochen".into(),
                "Sicherheitsstopp: inkrementeller Mirror wuerde zu viele Dateien loeschen.".into(),
            )],
            baseline: store.load_baseline(&pair).unwrap_or_default(),
            ..Default::default()
        });
    }
    let mut errors = Vec::new();
    let report = apply_with_results(
        &actions,
        a,
        root_a,
        b,
        root_b,
        opts,
        &versions_dir(&pair),
        &mut errors,
        cancel,
    );
    if cancel.load(Ordering::Relaxed) || !errors.is_empty() {
        return Some(Outcome {
            stats: report.stats,
            errors,
            baseline: store.load_baseline(&pair).unwrap_or_default(),
            ..Default::default()
        });
    }
    let mut updates = Vec::new();
    for ch in &changes {
        if let Some(old) = &ch.old_rel {
            updates.push(deleted_item(source_side, old, source_items.get(old)));
            updates.push(deleted_item(target_side, old, target_items.get(old)));
        }
        updates.push(source_item_after(source_side, ch));
        updates.push(target_item_after(target, target_root, target_side, ch));
    }
    if store.save_items(&pair, &updates).is_err() {
        return None;
    }
    if let Some(c) = new_cursor.as_deref() {
        let _ = store.update_cursor(&pair, Some(c));
    }
    let baseline = store.load_baseline(&pair).unwrap_or_default();
    let _ = save_baseline(&baseline_path(&pair), &baseline);
    Some(Outcome {
        stats: report.stats,
        conflicts: Vec::<Conflict>::new(),
        errors,
        baseline,
    })
}

pub(super) fn bootstrap_incremental_state(
    a: &dyn Backend,
    root_a: &str,
    b: &dyn Backend,
    root_b: &str,
    opts: BisyncOptions,
    baseline: &Baseline,
    source_cursor: Option<String>,
    store_path: Option<&Path>,
) -> rusqlite::Result<()> {
    let Some((_, _, source_side)) = mirror_source(a, root_a, b, root_b, opts) else {
        return Ok(());
    };
    let pair = pair_id(root_a, root_b);
    let mut store = open_store(store_path)?;
    let rec = PairRecord {
        pair: pair.clone(),
        root_a: root_a.into(),
        root_b: root_b.into(),
        mode: "mirror".into(),
        source_side,
        source_cursor,
        root_a_id: a.change_root_id(root_a).ok().flatten(),
        root_b_id: b.change_root_id(root_b).ok().flatten(),
        bootstrapped: true,
        target_managed: true,
    };
    store.save_pair(&rec)?;
    let ids_a = collect_ids(a, root_a, baseline, Side::A);
    let ids_b = collect_ids(b, root_b, baseline, Side::B);
    store.replace_from_baseline(&pair, baseline, &ids_a, &ids_b)
}

fn open_store(path: Option<&Path>) -> rusqlite::Result<SyncStateStore> {
    path.map_or_else(SyncStateStore::open_default, SyncStateStore::open_at)
}

fn record_matches(rec: &PairRecord, root_a: &str, root_b: &str, source_side: Side) -> bool {
    rec.root_a == root_a
        && rec.root_b == root_b
        && rec.mode == "mirror"
        && rec.source_side == source_side
        && rec.bootstrapped
        && rec.target_managed
}

fn root_id_matches(be: &dyn Backend, root: &str, saved: Option<&str>) -> bool {
    saved.map_or(true, |id| {
        be.change_root_id(root).ok().flatten().as_deref() == Some(id)
    })
}

fn changes_from_backend(
    store: &SyncStateStore,
    rec: &PairRecord,
    source: &dyn Backend,
    root: &str,
    side: Side,
) -> Option<(Vec<ResolvedChange>, Option<String>)> {
    let cursor = rec.source_cursor.as_deref()?;
    let batch = source.changes_since(root, cursor).ok()?;
    if batch.reset {
        return None;
    }
    let mut out = Vec::new();
    for raw in batch.changes {
        out.push(resolve_change(store, &rec.pair, side, rec, raw)?);
    }
    Some((out, batch.new_cursor))
}

fn changes_from_source_walk(
    source: &dyn Backend,
    root: &str,
    target: &dyn Backend,
    opts: BisyncOptions,
    filter: &WalkFilter,
    source_items: &BTreeMap<String, ItemRecord>,
) -> Option<(Vec<ResolvedChange>, Option<String>)> {
    let prev_tree: Tree = source_items
        .iter()
        .filter_map(|(rel, item)| {
            (!item.deleted)
                .then_some(item.sig)
                .flatten()
                .map(|s| (rel.clone(), s))
        })
        .collect();
    let cancel = AtomicBool::new(false);
    let mode = hash_mode(source, target, opts.compare);
    let cur = walk_files(source, root, &cancel, filter, mode, Some(&prev_tree)).ok()?;
    let mut rels: BTreeSet<String> = prev_tree.keys().cloned().collect();
    rels.extend(cur.keys().cloned());
    let mut changes = Vec::new();
    for rel in rels {
        let now = cur.get(&rel).copied();
        let prev = prev_tree.get(&rel).copied();
        if sig_eq(now, prev, &opts) {
            continue;
        }
        changes.push(ResolvedChange {
            rel: rel.clone(),
            old_rel: None,
            kind: now.map_or(ChangeKind::Remove, |_| ChangeKind::Upsert),
            id: source.item_id(&join(root, &rel)).ok().flatten(),
            parent_id: None,
            name: rel.rsplit('/').next().map(|s| s.to_string()),
            source_sig: now,
        });
    }
    Some((changes, None))
}

fn resolve_change(
    store: &SyncStateStore,
    pair: &str,
    side: Side,
    rec: &PairRecord,
    raw: VfsChange,
) -> Option<ResolvedChange> {
    let id_rel = raw
        .id
        .as_deref()
        .and_then(|id| store.rel_for_id(pair, side, id).ok().flatten());
    let parent_rel = rel_from_parent(store, pair, side, rec, &raw);
    let parent_addressed = raw.parent_id.is_some() && raw.name.is_some();
    let rel = raw
        .rel
        .clone()
        .or(parent_rel)
        .or_else(|| (!parent_addressed).then(|| id_rel.clone()).flatten())?;
    let old_rel = id_rel.filter(|old| old != &rel);
    let source_sig = raw.meta.as_ref().and_then(sig_from_meta);
    Some(ResolvedChange {
        rel,
        old_rel,
        kind: raw.kind,
        id: raw.id,
        parent_id: raw.parent_id,
        name: raw.name,
        source_sig,
    })
}

fn rel_from_parent(
    store: &SyncStateStore,
    pair: &str,
    side: Side,
    rec: &PairRecord,
    raw: &VfsChange,
) -> Option<String> {
    let parent_id = raw.parent_id.as_deref()?;
    let name = raw.name.as_deref()?;
    let root_id = if side == Side::A {
        rec.root_a_id.as_deref()
    } else {
        rec.root_b_id.as_deref()
    };
    let parent = if root_id == Some(parent_id) {
        Some(String::new())
    } else {
        store.rel_for_id(pair, side, parent_id).ok().flatten()
    }?;
    Some(if parent.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", parent, name)
    })
}

fn actions_for(source_side: Side, changes: &[ResolvedChange]) -> Vec<Action> {
    let mut out = Vec::new();
    for ch in changes {
        if let (ChangeKind::Upsert, Some(old)) = (&ch.kind, &ch.old_rel) {
            out.push(match source_side {
                Side::A => Action::DeleteB(old.clone()),
                Side::B => Action::DeleteA(old.clone()),
            });
        }
        out.push(match (source_side, &ch.kind) {
            (Side::A, ChangeKind::Upsert) => Action::CopyAtoB(ch.rel.clone()),
            (Side::A, ChangeKind::Remove) => Action::DeleteB(ch.rel.clone()),
            (Side::B, ChangeKind::Upsert) => Action::CopyBtoA(ch.rel.clone()),
            (Side::B, ChangeKind::Remove) => Action::DeleteA(ch.rel.clone()),
        });
    }
    out
}

fn target_touched_drifted(
    target: &dyn Backend,
    root: &str,
    target_items: &BTreeMap<String, ItemRecord>,
    changes: &[ResolvedChange],
    opts: BisyncOptions,
) -> bool {
    for ch in changes {
        if target_rel_drifted(target, root, target_items, &ch.rel, opts) {
            return true;
        }
        if ch
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
    let actual = target
        .stat(&join(root, rel))
        .ok()
        .and_then(|m| sig_from_meta(&m));
    !sig_eq(actual, expected, &opts)
}

fn delete_guard_trips(
    actions: &[Action],
    target_items: &BTreeMap<String, ItemRecord>,
    opts: BisyncOptions,
) -> bool {
    let deletes = actions
        .iter()
        .filter(|a| matches!(a, Action::DeleteA(_) | Action::DeleteB(_)))
        .count() as u64;
    if deletes == 0 {
        return false;
    }
    let total = target_items.values().filter(|i| !i.deleted).count() as u64;
    (opts.max_delete > 0 && deletes > opts.max_delete)
        || (opts.max_delete_pct > 0 && deletes > total * opts.max_delete_pct as u64 / 100)
}

fn source_item_after(side: Side, ch: &ResolvedChange) -> ItemRecord {
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

fn target_item_after(
    target: &dyn Backend,
    root: &str,
    side: Side,
    ch: &ResolvedChange,
) -> ItemRecord {
    let path = join(root, &ch.rel);
    let meta = target.stat(&path).ok();
    ItemRecord {
        side,
        rel: ch.rel.clone(),
        id: target
            .item_id(&path)
            .ok()
            .flatten()
            .or_else(|| meta.as_ref().and_then(|m| m.id.clone())),
        parent_id: parent_id_for(target, root, &ch.rel),
        name: ch
            .name
            .clone()
            .or_else(|| ch.rel.rsplit('/').next().map(|s| s.to_string())),
        sig: meta.as_ref().and_then(sig_from_meta),
        is_dir: false,
        deleted: ch.kind == ChangeKind::Remove || meta.is_none(),
    }
}

fn deleted_item(side: Side, rel: &str, prev: Option<&ItemRecord>) -> ItemRecord {
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
    if m.is_dir {
        return None;
    }
    Some(Sig {
        size: m.size,
        mtime_ms: m.mtime_ms,
        hash: m.content_md5.as_deref().map(md5_hex_to_u64).unwrap_or(0),
    })
}

fn collect_ids(
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
