//! Cache retirement and complete index persistence; checkpoints remain authoritative.
use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::bisync as engine;
use engine::incremental_changes::collect_ids;
use engine::orchestration::RunState;
use engine::replica_state::index_id;
use engine::state_metadata::{index_dirty_path, write_bytes};
use engine::state_store::{PairRecord, Side, SyncStateStore};
use engine::types::{Baseline, BisyncOptions};
use super::{mirror_source, SyncEndpoints};

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
    let names =
        engine::state_spellings::load(state.key, keys).map_err(|_| rusqlite::Error::InvalidQuery)?;
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
