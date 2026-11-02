//! Exact-ID mirror cleanup with one normalized plan index for the whole list.
use super::apply::ApplyReport;
use super::apply_guard::{capture, drift, revalidate, ExpectedFile};
use super::checkpoint::ApplyScope;
use super::completion::{CompletedAction, CompletedKind};
use super::duplicate_observation::{observe_named, verify_named};
use super::incremental::SyncEndpoints;
use super::keys::KeyPolicy;
use super::types::{Action, BisyncOptions, PairSide, Tree};
use super::versions::VersionSide;
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    candidates: &[crate::vfs::DedupeCandidate],
    side: PairSide,
    a: &Tree,
    b: &Tree,
    endpoints: SyncEndpoints<'_>,
    opts: BisyncOptions,
    scope: &ApplyScope<'_>,
    errors: &mut Vec<(String, String)>,
    cancel: &AtomicBool,
) -> ApplyReport {
    let mut report = ApplyReport::default();
    if opts.dry_run {
        report.stats.deleted = candidates.len() as u64;
        return report;
    }
    let (backend, root, target, peer, peer_root, source) = if side == PairSide::A {
        (
            endpoints.a,
            endpoints.root_a,
            a,
            endpoints.b,
            endpoints.root_b,
            b,
        )
    } else {
        (
            endpoints.b,
            endpoints.root_b,
            b,
            endpoints.a,
            endpoints.root_a,
            a,
        )
    };
    let keys = KeyPolicy::for_pair(
        endpoints.a.case_sensitive_paths(endpoints.root_a),
        endpoints.b.case_sensitive_paths(endpoints.root_b),
    );
    let targets: BTreeMap<_, _> = target
        .iter()
        .map(|(rel, sig)| (keys.key(rel).into_owned(), (rel.as_str(), *sig)))
        .collect();
    let sources: BTreeMap<_, _> = source
        .iter()
        .map(|(rel, sig)| (keys.key(rel).into_owned(), (rel.as_str(), *sig)))
        .collect();
    let mut literal_paths = BTreeMap::new();
    for rel in target.keys() {
        match crate::vfs::sync_path(backend, root, rel) {
            Ok(path) => {
                literal_paths.insert(path, rel.as_str());
            }
            Err(error) => {
                report.stats.errors += 1;
                errors.push((rel.clone(), error.to_string()));
            }
        }
    }
    let mut groups: BTreeMap<String, Vec<&crate::vfs::DedupeCandidate>> = BTreeMap::new();
    let prefix = format!("{}/", root.trim_end_matches('/'));
    for candidate in candidates {
        let Some(_) = candidate.path.strip_prefix(&prefix) else {
            report.stats.errors += 1;
            errors.push((
                candidate.path.clone(),
                "duplicate candidate is outside the sync root".into(),
            ));
            continue;
        };
        let Some(rel) = literal_paths.get(&candidate.path) else {
            // The provider path is encoded; never treat its bytes as a
            // new literal relative name or infer absence from that mismatch.
            scope.sink.deferred(
                &candidate.path,
                "duplicate candidate has no observed literal path",
            );
            continue;
        };
        groups
            .entry(keys.key(rel).into_owned())
            .or_default()
            .push(candidate);
    }
    let stopped = AtomicBool::new(false);
    if let Err(error) = scope.versions.bind_lock(&super::pair_lock_id(
        endpoints.a,
        endpoints.root_a,
        endpoints.b,
        endpoints.root_b,
    )) {
        report.stats.errors += 1;
        errors.push(("Versionen".into(), error.to_string()));
        return report;
    }
    let version_side = VersionSide {
        side,
        backend,
        root,
    };
    for (key, planned) in groups {
        if cancel.load(Ordering::Acquire)
            || scope.sink.should_stop()
            || stopped.load(Ordering::Acquire)
        {
            break;
        }
        let raw_rel = planned[0].path.strip_prefix(&prefix).unwrap_or_default();
        let rel = targets
            .get(&key)
            .map(|(rel, _)| *rel)
            .or_else(|| sources.get(&key).map(|(rel, _)| *rel))
            .unwrap_or(raw_rel);
        let result = (|| {
            let target_rel = scope.spellings.side_rel(rel, side);
            let source_rel = sources
                .get(&key)
                .map(|(rel, _)| *rel)
                .unwrap_or_else(|| scope.spellings.side_rel(rel, side.other()));
            super::apply_boundary::guard(backend, root, target_rel, opts.cross_mounts)?;
            super::apply_boundary::guard(peer, peer_root, source_rel, opts.cross_mounts)?;
            let path = crate::vfs::sync_path(backend, root, target_rel)?;
            let peer_path = crate::vfs::sync_path(peer, peer_root, source_rel)?;
            let peer_expected = sources.get(&key).map_or(ExpectedFile::Missing, |(_, sig)| {
                ExpectedFile::Present(*sig)
            });
            let peer_state = capture(peer, &peer_path, peer_expected, "mirror source")?;
            let name = target_rel.rsplit('/').next().unwrap_or(target_rel);
            let metas = super::duplicate_observation::metadata_named(backend, &path, name)?;
            if metas.iter().any(|meta| meta.special) {
                return Err(super::apply_boundary::protected(
                    super::OmissionKind::Special,
                ));
            }
            let mut variants = observe_named(backend, &path, name, Some(&metas), cancel)?;
            let mut ids = BTreeSet::new();
            for candidate in &planned {
                if backend.has_duplicate_file_names() && candidate.id.is_none() {
                    return Err(drift("duplicate candidate has no exact ID"));
                }
                if !ids.insert(candidate.id.clone()) {
                    return Err(drift("duplicate ID was planned twice"));
                }
                if !variants.iter().any(|variant| variant.id == candidate.id) {
                    return Err(drift("duplicate object changed since planning"));
                }
            }
            let keep: Option<super::duplicate_types::FileVariant> = if let Some((_, expected)) =
                targets.get(&key).filter(|_| sources.contains_key(&key))
            {
                variants
                    .iter()
                    .find(|variant| {
                        !ids.contains(&variant.id)
                            && variant.signature.size == expected.size
                            && variant.signature.mtime_ms == expected.mtime_ms
                            && (expected.hash == 0 || variant.signature.hash == expected.hash)
                    })
                    .cloned()
                    .ok_or_else(|| drift("planned duplicate survivor changed"))?
                    .into()
            } else {
                None
            };
            if !sources.contains_key(&key) && ids.len() != variants.len() {
                return Err(drift("orphan duplicate group changed since planning"));
            }
            if sources.contains_key(&key) && keep.is_none() {
                return Err(drift(
                    "mirror cleanup would remove the source's current name",
                ));
            }
            if opts.reversible {
                for variant in variants.iter().filter(|variant| ids.contains(&variant.id)) {
                    super::apply_transaction::gate(cancel, Some(scope.sink))?;
                    super::duplicate_backup::save_scoped(
                        &version_side,
                        &path,
                        target_rel,
                        scope.versions,
                        variant,
                        cancel,
                    )?;
                }
            }
            for candidate in planned {
                super::apply_transaction::gate(cancel, Some(scope.sink))?;
                verify_named(backend, &path, name, &variants, cancel)?;
                revalidate(peer, &peer_path, &peer_state, "mirror source")?;
                if peer_state.metadata.is_none()
                    && !super::apply_boundary::normalized_missing(
                        peer,
                        peer_root,
                        source_rel,
                        keys,
                        opts.cross_mounts,
                        cancel,
                    )?
                {
                    return Err(drift("mirror source appeared under another spelling"));
                }
                backend.remove_file_id(&path, candidate.id.as_deref())?;
                super::apply_stage::require_durable(super::apply_stage::namespace(
                    backend, &path,
                )?)?;
                variants.retain(|variant| variant.id != candidate.id);
                report.stats.deleted += 1;
                // A survivor must retain its signature; deleting an extra
                // ID must never clear the entire logical file's baseline.
                if let Some(keep) = &keep {
                    scope.sink.completed(CompletedAction {
                        rel: rel.to_string(),
                        kind: CompletedKind::Copied { from: side.other() },
                        src_sig: sources.get(&key).map(|(_, sig)| *sig),
                        dst_sig: Some(super::Sig {
                            hash: if matches!(
                                super::snapshot_hash::hash_mode(backend, peer, opts.compare),
                                super::snapshot_hash::HashMode::None
                            ) {
                                0
                            } else {
                                keep.signature.hash
                            },
                            ..keep.signature
                        }),
                        durable: true,
                    });
                } else if variants.is_empty() {
                    scope.sink.completed(CompletedAction {
                        rel: rel.to_string(),
                        kind: CompletedKind::Deleted { side },
                        src_sig: None,
                        dst_sig: None,
                        durable: true,
                    });
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => report.completed.push(if side == PairSide::A {
                Action::DeleteA(rel.to_string())
            } else {
                Action::DeleteB(rel.to_string())
            }),
            Err(error) => {
                if !super::apply_reporting::classify(&error, rel, side, scope, &stopped) {
                    report.stats.errors += 1;
                    if errors.len() < 100 {
                        errors.push((rel.to_string(), error.to_string()));
                    }
                }
                scope.sink.omitted(rel, super::OmissionKind::Unreadable);
            }
        }
    }
    report
}
