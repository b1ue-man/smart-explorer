//! Interactive single-file work uses the same guards and durable journal as
//! a run. Its baseline is merged under the pair lock, never replaced by a UI.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use super::checkpoint::{ApplyScope, CollectingSink};
use super::checkpoint_run::CheckpointSink;
use super::incremental::SyncEndpoints;
use super::keys::Spellings;
use super::pair_lock::PairLock;
use super::run_types::{RunSettings, StateKey};
use super::state_metadata::{load_history, save_history, PairHistory};
use super::types::{Action, BisyncOptions, BisyncStats, PairSide, Sig, Tree};

pub(super) fn validate_state(endpoints: SyncEndpoints<'_>, key: &StateKey) -> io::Result<()> {
    crate::vfs::validate_sync_roots(endpoints.a, endpoints.root_a, endpoints.b, endpoints.root_b)?;
    if key.pair_id
        != super::pair_id_for(endpoints.a, endpoints.root_a, endpoints.b, endpoints.root_b)
        || key.lock_id
            != super::pair_lock_id(endpoints.a, endpoints.root_a, endpoints.b, endpoints.root_b)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Vorschau gehört zu einem anderen Sync-Paar",
        ));
    }
    let settings = RunSettings {
        owner: key.owner.clone(),
        ..RunSettings::default()
    };
    let replicas = super::replica::identify(endpoints, &settings, false)?;
    if let Some(block) = replicas.blocked {
        return Err(io::Error::new(io::ErrorKind::InvalidData, block.message()));
    }
    if replicas.key != *key {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Laufwerk seit der Vorschau gewechselt; bitte neu vergleichen",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_one(
    endpoints: SyncEndpoints<'_>,
    lock: &PairLock,
    key: &StateKey,
    action: &Action,
    expected: (Option<Sig>, Option<Sig>),
    spellings: &Spellings,
    mut opts: BisyncOptions,
    cancel: &AtomicBool,
) -> io::Result<(BisyncStats, (Option<Sig>, Option<Sig>))> {
    if cancel.load(Ordering::Acquire) {
        return Err(interrupted());
    }
    let rel = super::core::action_rel(action);
    crate::agent_proto::ValidatedRelativePath::parse(rel)?;
    let keys = super::orchestration_plan::keys(endpoints);
    let mut protected = super::SyncOmissions::new(keys.fold_case);
    for pending in super::orchestration_plan::pending_paths(lock, key, endpoints)? {
        protected.record_kind(&pending, super::OmissionKind::Unreadable, false);
    }
    if protected.protects(rel) {
        return Err(io::Error::new(io::ErrorKind::WouldBlock,
            "Für diesen Pfad ist ein geschützter Wiederanlauf offen; bitte den betreffenden Lauf fortsetzen"));
    }
    let mut a = Tree::new();
    let mut b = Tree::new();
    for (side, signature, tree) in [
        (PairSide::A, expected.0, &mut a),
        (PairSide::B, expected.1, &mut b),
    ] {
        let path = spellings.side_rel(rel, side);
        crate::agent_proto::ValidatedRelativePath::parse(path)?;
        if let Some(signature) = signature {
            tree.insert(path.to_string(), signature);
        }
    }
    let mut names = super::state_spellings::load(key, keys)?;
    let history = load_history(key)?;
    let collected = CollectingSink::default();
    let sink = CheckpointSink::new(endpoints, lock, key, keys, Some(&collected))?;
    opts.dry_run = false;
    let versions = super::versions::RunVersions::begin(super::versions::VersionsContext::new(
        &key.pair_id,
        key.owner.clone(),
        opts.versions,
        opts.versioning,
    ));
    let scope = ApplyScope {
        sink: &sink,
        versions: &versions,
        spellings,
    };
    let mut errors = Vec::new();
    let report = sink.during(|| {
        super::apply::apply_planned_reporting(
            std::slice::from_ref(action),
            &[],
            &a,
            &b,
            endpoints,
            opts,
            &scope,
            &mut errors,
            cancel,
        )
    });
    let checkpoint = sink.finish();
    let events = collected.take();
    // Finish versions after any attempted action, including failures/cancel.
    let versions_result = versions.finish().and_then(|()| {
        super::versions::prune_after_run(
            lock,
            &key.pair_id,
            &[
                super::versions::VersionSide {
                    side: PairSide::A,
                    backend: endpoints.a,
                    root: endpoints.root_a,
                },
                super::versions::VersionSide {
                    side: PairSide::B,
                    backend: endpoints.b,
                    root: endpoints.root_b,
                },
            ],
            &opts.versioning,
            &AtomicBool::new(false),
        )
    });
    if let Some(error) = checkpoint.error {
        return Err(io::Error::other(error));
    }
    let result = events
        .completed
        .iter()
        .rev()
        .find(|entry| entry.rel == rel)
        .and_then(|entry| entry.baseline_entry());
    if let Some(result) = result {
        names.applied(
            std::slice::from_ref(action),
            spellings,
            &checkpoint.baseline,
            keys,
        );
        super::state_spellings::save(key, &names)?;
        let mut counts = history
            .as_ref()
            .map(|history| [history.entries_a, history.entries_b])
            .unwrap_or_else(|| {
                [
                    super::guards::recorded_entries(&checkpoint.baseline, PairSide::A),
                    super::guards::recorded_entries(&checkpoint.baseline, PairSide::B),
                ]
            });
        if history.is_some() {
            for (index, before, after) in [(0, expected.0, result.0), (1, expected.1, result.1)] {
                if before.is_some() {
                    counts[index] = counts[index].max(1);
                }
                if before.is_none() && after.is_some() {
                    counts[index] = counts[index].saturating_add(1);
                }
                if before.is_some() && after.is_none() {
                    counts[index] = counts[index].saturating_sub(1);
                }
            }
        }
        save_history(
            key,
            &PairHistory {
                replica_a: key.replica_a.clone(),
                replica_b: key.replica_b.clone(),
                entries_a: counts[0],
                entries_b: counts[1],
                full_ms: history.as_ref().map_or(0, |history| history.full_ms),
            },
        )?;
    }
    versions_result?;
    if report.stats.errors > 0 {
        return Err(io::Error::other(errors.first().map_or_else(
            || "Datei konnte nicht übernommen werden".into(),
            |(_, error)| error.clone(),
        )));
    }
    if let Some(stop) = checkpoint.stopped {
        return Err(io::Error::other(stop.message()));
    }
    if let Some((_, reason)) = checkpoint.deferred.first() {
        return Err(io::Error::new(io::ErrorKind::WouldBlock, reason.clone()));
    }
    if let Some((_, kind)) = checkpoint.omitted.first() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, kind.label()));
    }
    match result {
        Some(result) => Ok((report.stats, result)),
        None if cancel.load(Ordering::Acquire) => Err(interrupted()),
        None => Err(io::Error::other(
            "Apply hat kein abgeschlossenes Dateiergebnis gemeldet",
        )),
    }
}

fn interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Dateiübernahme abgebrochen")
}
