//! Cache retirement and complete index persistence; checkpoints remain authoritative.
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;
use std::sync::atomic::Ordering;

use sha2::{Digest, Sha256};

use super::{mirror_source, SyncEndpoints};
use crate::bisync as engine;
use engine::incremental_changes::collect_ids;
use engine::orchestration::RunState;
use engine::replica_state::index_id;
use engine::state_metadata::{index_dirty_path, write_bytes};
use engine::state_store::{ItemRecord, PairRecord, Side, SyncStateStore};
use engine::types::{Baseline, BisyncOptions, Sig};

/// A durable dirty marker disqualifies the old cache even if SQLite itself is
/// corrupt, read-only or busy. A full scan can then work without the database.
pub(in crate::bisync) fn retire_index(state: &RunState<'_>) -> io::Result<()> {
    if state.opts.dry_run || mirror_source(state.endpoints, state.opts).is_none() {
        return Ok(());
    }
    write_bytes(&index_dirty_path(state.key)?, b"full planner required\n")?;
    if let (Ok(pair), Ok(mut store)) = (index_id(state.key), open_store(state.store_path)) {
        let _ = store.forget_pair(&pair);
    }
    Ok(())
}

pub(in crate::bisync) fn bootstrap_run(
    state: &RunState<'_>,
    baseline: &Baseline,
    cursor: Option<String>,
) -> rusqlite::Result<()> {
    if !engine::orchestration_plan::pending_paths(state.lock, state.key, state.endpoints)
        .map_err(|_| rusqlite::Error::InvalidQuery)?
        .is_empty()
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let Some((_, _, source_side)) = mirror_source(state.endpoints, state.opts) else {
        return Ok(());
    };
    let keys = engine::orchestration_plan::keys(state.endpoints);
    let names = engine::state_spellings::load(state.key, keys)
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    let rows = names.cache_baseline(baseline, keys);
    let pair = index_id(state.key).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let record = pair_record(state.endpoints, pair, mode(state), source_side, cursor);
    engine::engine_change_feed::bootstrap(
        &mut open_store(state.store_path)?,
        &record,
        state.endpoints,
        &rows,
        state.opts,
        state.filter,
        state.cancel,
    )
    .map_err(|_| rusqlite::Error::InvalidQuery)?;
    std::fs::remove_file(index_dirty_path(state.key).map_err(|_| rusqlite::Error::InvalidQuery)?)
        .map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(())
}

/// Extend a previously complete generation with confirmed action deltas.
/// Initial/partial observations still need bootstrap_run's complete walks.
pub(super) fn commit_incremental(
    state: &RunState<'_>,
    baseline: &Baseline,
    previous: &PairRecord,
    items: [&BTreeMap<String, ItemRecord>; 2],
    changes: &[engine::incremental_collect::ResolvedChange],
    cursor: Option<String>,
    aliases: &engine::keys::PathAliases,
) -> rusqlite::Result<()> {
    let invalid = || rusqlite::Error::InvalidQuery;
    let (_, _, source_side) = mirror_source(state.endpoints, state.opts).ok_or_else(invalid)?;
    let keys = engine::orchestration_plan::keys(state.endpoints);
    if previous.pair != index_id(state.key).map_err(|_| invalid())?
        || !super::record_matches(previous, state.endpoints, source_side)
        || previous.mode != mode(state)
        || engine::state_spelling_aliases::cache_by_key(items[0], items[1], keys, aliases)
            != Some(engine::state_spelling_aliases::baseline_by_key(
                state.baseline,
                keys,
            ))
        || !super::root_id_matches(
            state.endpoints.a,
            state.endpoints.root_a,
            previous.root_a_id.as_deref(),
        )
        || !super::root_id_matches(
            state.endpoints.b,
            state.endpoints.root_b,
            previous.root_b_id.as_deref(),
        )
        || !engine::orchestration_plan::pending_paths(state.lock, state.key, state.endpoints)
            .map_err(|_| invalid())?
            .is_empty()
    {
        return Err(invalid());
    }
    let names = engine::state_spellings::load(state.key, keys).map_err(|_| invalid())?;
    let confirmed = names.cache_baseline(baseline, keys);
    let source_pair = if source_side == Side::A {
        engine::PairSide::A
    } else {
        engine::PairSide::B
    };
    let touched: BTreeSet<_> = changes
        .iter()
        .flat_map(|change| std::iter::once(change.rel.as_str()).chain(change.old_rel.as_deref()))
        .map(|rel| aliases.key(rel, source_pair, keys))
        .collect();
    let mut rows = Vec::new();
    let mut budget = engine::state_validation::StateBudget::for_pair();
    for (index, side, backend, root, root_id) in [
        (
            0,
            Side::A,
            state.endpoints.a,
            state.endpoints.root_a,
            previous.root_a_id.as_deref(),
        ),
        (
            1,
            Side::B,
            state.endpoints.b,
            state.endpoints.root_b,
            previous.root_b_id.as_deref(),
        ),
    ] {
        let before = items[index];
        let mut ids = BTreeSet::new();
        if let Some(root_id) = root_id {
            ids.insert(root_id.to_string());
        }
        for item in before.values().filter(|item| !item.is_dir && !item.deleted) {
            let now =
                confirmed.get(&item.rel).and_then(
                    |entry| {
                        if side == Side::A {
                            entry.0
                        } else {
                            entry.1
                        }
                    },
                );
            let pair_side = if side == Side::A {
                engine::PairSide::A
            } else {
                engine::PairSide::B
            };
            if !touched.contains(&aliases.key(&item.rel, pair_side, keys)) && now != item.sig {
                return Err(invalid());
            }
        }
        for item in before.values().filter(|item| item.is_dir && !item.deleted) {
            if state.cancel.load(Ordering::Acquire)
                || item.id.as_ref().is_some_and(|id| !ids.insert(id.clone()))
            {
                return Err(invalid());
            }
            budget.record_item(item)?;
            rows.push(item.clone());
        }
        for (rel, entry) in &confirmed {
            let Some(signature) = (if side == Side::A { entry.0 } else { entry.1 }) else {
                continue;
            };
            if state.cancel.load(Ordering::Acquire) {
                return Err(invalid());
            }
            let pair_side = if side == Side::A {
                engine::PairSide::A
            } else {
                engine::PairSide::B
            };
            let item = if touched.contains(&aliases.key(rel, pair_side, keys)) {
                confirmed_item(state, side, backend, root, rel, signature).map_err(|_| invalid())?
            } else {
                before
                    .get(rel)
                    .filter(|item| !item.deleted && !item.is_dir && item.sig == Some(signature))
                    .cloned()
                    .ok_or_else(invalid)?
            };
            let parent = rel.rsplit_once('/').map_or("", |(parent, _)| parent);
            let parent_id = if parent.is_empty() {
                root_id
            } else {
                before
                    .get(parent)
                    .filter(|item| item.is_dir && !item.deleted)
                    .ok_or_else(invalid)?
                    .id
                    .as_deref()
            };
            if parent_id.is_some_and(|id| item.parent_id.as_deref() != Some(id))
                || (backend.supports_changes() && (item.id.is_none() || item.parent_id.is_none()))
                || item.id.as_ref().is_some_and(|id| !ids.insert(id.clone()))
            {
                return Err(invalid());
            }
            budget.record_item(&item)?;
            rows.push(item);
        }
    }
    // The dirty marker remains until all rows and the cursor are atomically
    // stored. A failure/cancel selects the full planner on the next run.
    if state.cancel.load(Ordering::Acquire) {
        return Err(invalid());
    }
    let mut record = previous.clone();
    record.source_cursor = cursor;
    let mut store = open_store(state.store_path)?;
    let tx = store.conn.transaction()?;
    tx.execute("DELETE FROM items WHERE pair = ?1", [&record.pair])?;
    for item in &rows {
        engine::state_store::upsert_item_tx(&tx, &record.pair, item)?;
    }
    engine::state_store::write_pair(&tx, &record)?;
    tx.commit()?;
    if state.cancel.load(Ordering::Acquire) {
        return Err(invalid());
    }
    std::fs::remove_file(index_dirty_path(state.key).map_err(|_| invalid())?).map_err(|_| invalid())
}

fn confirmed_item(
    state: &RunState<'_>,
    side: Side,
    backend: &dyn crate::vfs::Backend,
    root: &str,
    rel: &str,
    signature: Sig,
) -> io::Result<ItemRecord> {
    use engine::apply_guard::{capture, revalidate, ExpectedFile};
    engine::apply_boundary::guard(backend, root, rel, state.opts.cross_mounts)?;
    let path = crate::vfs::sync_path(backend, root, rel)?;
    let captured = capture(
        backend,
        &path,
        ExpectedFile::Present(signature),
        "indexed result",
    )?;
    let meta = captured.regular("indexed result")?;
    if signature.hash != 0
        && meta
            .content_md5
            .as_deref()
            .is_none_or(|digest| engine::snapshot_hash::md5_hex_to_u64(digest) == 0)
        && engine::snapshot_hash::hash_file(backend, &path, state.cancel)? != signature.hash
    {
        return Err(engine::apply_guard::drift(
            "indexed result changed after checkpoint",
        ));
    }
    let id = backend.item_id(&path)?.or(meta.id.clone());
    if meta
        .id
        .as_ref()
        .is_some_and(|captured_id| id.as_ref() != Some(captured_id))
    {
        return Err(engine::apply_guard::drift(
            "indexed result identity changed",
        ));
    }
    let parent = rel.rsplit_once('/').map_or("", |(parent, _)| parent);
    let parent_id = backend.item_id(&crate::vfs::sync_path(backend, root, parent)?)?;
    revalidate(backend, &path, &captured, "indexed result")?;
    Ok(ItemRecord {
        side,
        rel: rel.to_string(),
        id,
        parent_id,
        name: Some(meta.name.clone()),
        sig: Some(signature),
        is_dir: false,
        deleted: false,
    })
}

// These cache-only entry points remain for existing integrations and fixtures.
pub(in crate::bisync) fn bootstrap_incremental_state(
    endpoints: SyncEndpoints<'_>,
    opts: BisyncOptions,
    baseline: &Baseline,
    cursor: Option<String>,
    path: Option<&Path>,
) -> rusqlite::Result<()> {
    let Some((_, _, side)) = mirror_source(endpoints, opts) else {
        return Ok(());
    };
    let pair = engine::pair_id_for(endpoints.a, endpoints.root_a, endpoints.b, endpoints.root_b);
    let record = pair_record(endpoints, pair, "mirror".into(), side, cursor);
    let ids_a = collect_ids(endpoints.a, endpoints.root_a, baseline, Side::A);
    let ids_b = collect_ids(endpoints.b, endpoints.root_b, baseline, Side::B);
    open_store(path)?.bootstrap(&record, baseline, &ids_a, &ids_b)
}

pub(in crate::bisync) fn invalidate_incremental_state(
    endpoints: SyncEndpoints<'_>,
    opts: BisyncOptions,
    path: Option<&Path>,
) -> rusqlite::Result<()> {
    if opts.dry_run || mirror_source(endpoints, opts).is_none() {
        return Ok(());
    }
    let pair = engine::pair_id_for(endpoints.a, endpoints.root_a, endpoints.b, endpoints.root_b);
    open_store(path)?.forget_pair(&pair)
}

pub(super) fn open_store(path: Option<&Path>) -> rusqlite::Result<SyncStateStore> {
    path.map_or_else(SyncStateStore::open_default, SyncStateStore::open_at)
}
fn pair_record(
    endpoints: SyncEndpoints<'_>,
    pair: String,
    mode: String,
    source_side: Side,
    cursor: Option<String>,
) -> PairRecord {
    PairRecord {
        pair,
        root_a: endpoints.root_a.into(),
        root_b: endpoints.root_b.into(),
        mode,
        source_side,
        source_cursor: cursor,
        root_a_id: endpoints.a.change_root_id(endpoints.root_a).ok().flatten(),
        root_b_id: endpoints.b.change_root_id(endpoints.root_b).ok().flatten(),
        bootstrapped: true,
        target_managed: true,
    }
}
pub(super) fn mode(state: &RunState<'_>) -> String {
    // Debug is an opaque cache fingerprint, never a persisted endpoint or a
    // protocol contract. A dependency update may invalidate it safely.
    let text = format!(
        "{:?}:{}:{}:{:?}:{}:{}:{}:{}:{}",
        state.opts.compare,
        state.opts.modify_window_ms,
        state.opts.cross_mounts,
        state.filter.ignore,
        state.filter.include_hidden,
        state.filter.min_size,
        state.filter.max_size,
        state.filter.after_mtime_ms,
        state.filter.before_mtime_ms
    );
    format!("mirror-rv2-ancestry:{:x}", Sha256::digest(text.as_bytes()))
}
