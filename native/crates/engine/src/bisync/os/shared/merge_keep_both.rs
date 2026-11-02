//! Publish matching loser siblings before replacing either original.
use super::apply_guard::{capture, drift, revalidate, ExpectedFile};
use super::incremental::SyncEndpoints;
use super::merge_precheck::Compared;
use super::merge_recorded::{MergeFile, MergeReport};
use super::merge_recovery::{self, Recovery};
use super::run_types::StateKey;
use super::types::Sig;
use std::io;
use std::sync::atomic::AtomicBool;

#[allow(clippy::too_many_arguments)]
pub(super) fn preserve_both(
    endpoints: SyncEndpoints<'_>,
    original_rel: &str,
    original_spellings: [&str; 2],
    bytes: &[u8],
    cross_mounts: bool,
    key: &StateKey,
    recovery: &mut Recovery,
    report: &mut MergeReport,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let SyncEndpoints {
        a,
        root_a,
        b,
        root_b,
    } = endpoints;
    if recovery.sibling.is_none() {
        let suffix = super::version_manifest::token(&recovery.run);
        let parent = original_rel.rsplit_once('/').map(|(parent, _)| parent);
        let mut name = original_rel
            .rsplit('/')
            .next()
            .unwrap_or(original_rel)
            .to_string();
        let suffix = format!(" (Konflikt {})", &suffix[..12]);
        loop {
            let candidate = format!("{name}{suffix}");
            if [
                crate::vfs::target_limits(a, root_a),
                crate::vfs::target_limits(b, root_b),
            ]
            .iter()
            .all(|limits| limits.name_issue(&candidate).is_none())
            {
                recovery.sibling = Some(
                    parent.map_or(candidate.clone(), |parent| format!("{parent}/{candidate}")),
                );
                break;
            }
            if name.pop().is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidFilename,
                    "no storable conflict sibling",
                ));
            }
        }
        merge_recovery::save(key, recovery)?;
    }
    let rel = recovery
        .sibling
        .clone()
        .ok_or_else(|| drift("missing conflict sibling"))?;
    let mut item = MergeFile {
        rel: rel.clone(),
        a: recovery.sibling_a,
        b: recovery.sibling_b,
    };
    for (index, backend, root) in [(0, a, root_a), (1, b, root_b)] {
        super::transfer_stream::check(cancel)?;
        let side_rel = sibling_spelling(&rel, original_spellings[index]);
        super::apply_boundary::guard(backend, root, &side_rel, cross_mounts)?;
        super::apply_boundary::target(backend, root, &side_rel, Some(bytes.len() as u64))?;
        let path = crate::vfs::sync_path(backend, root, &side_rel)?;
        let prior = if index == 0 {
            recovery.sibling_a
        } else {
            recovery.sibling_b
        };
        let current = capture(backend, &path, ExpectedFile::Unknown, "merge sibling")?;
        if let Some(meta) = &current.metadata {
            current.regular("merge sibling")?;
            // Recovery can adopt an uncertain create only after exact bytes
            // and real namespace confirmation, never by mere size equality.
            let mut reader = crate::vfs::open_read_regular(backend, &path, meta.id.as_deref())?;
            let mut compared = Compared::new(bytes, bytes);
            let checked = super::transfer_stream::stream(
                &mut *reader,
                &mut compared,
                cancel,
                None,
                0,
                |_| {},
            )?;
            if checked.bytes != bytes.len() as u64 || !compared.first_matches() {
                return Err(drift("conflict sibling already exists with other content"));
            }
            revalidate(backend, &path, &current, "merge sibling")?;
            super::apply_stage::require_durable(super::apply_stage::namespace(backend, &path)?)?;
            let sig = Sig {
                size: meta.size,
                mtime_ms: meta.mtime_ms,
                hash: checked.hash(),
            };
            if index == 0 {
                item.a = Some(sig);
                recovery.sibling_a = item.a;
            } else {
                item.b = Some(sig);
                recovery.sibling_b = item.b;
            }
        } else {
            if prior.is_some() {
                return Err(drift("confirmed conflict sibling disappeared"));
            }
            let stage = super::apply_stage::stage_bytes(
                backend,
                &path,
                &current,
                bytes,
                super::versions::now_ms(),
                cancel,
            )?;
            let outcome = stage.publish(&path, &current, true, cancel)?;
            super::apply_stage::require_durable(outcome.durable)?;
            if index == 0 {
                item.a = Some(outcome.destination);
                recovery.sibling_a = item.a;
            } else {
                item.b = Some(outcome.destination);
                recovery.sibling_b = item.b;
            }
        }
        report.preserved = vec![item.clone()];
        merge_recovery::save(key, recovery)?;
    }
    Ok(())
}

pub(super) fn sibling_spelling(rel: &str, original: &str) -> String {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    original.rsplit_once('/').map_or_else(
        || name.to_string(),
        |(parent, _)| format!("{parent}/{name}"),
    )
}
