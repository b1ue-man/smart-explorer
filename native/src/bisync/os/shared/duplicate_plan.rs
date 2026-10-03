use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;
use super::duplicate_observation::observe;
use super::duplicate_types::DuplicateConflict;
use super::incremental::SyncEndpoints;
use super::omissions::{OmissionKind, SyncOmissions};
use super::paths::join;
use super::snapshot_duplicates::DuplicateGroups;
use super::types::{BisyncOptions, Conflict, DeletePolicy, Tree};

pub(super) fn check_post_scan(snapshot: super::snapshot::Snapshot, conflicts: &[Conflict])
    -> std::io::Result<super::snapshot::Snapshot> {
    if snapshot.duplicates.keys().any(|rel| !conflicts.iter().any(|c| &c.rel == rel)) {
        return Err(super::duplicate_observation::changed());
    }
    Ok(snapshot)
}

/// Duplicate observations never become absence or separate alias paths.
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare(
    endpoints: SyncEndpoints<'_>, groups_a: DuplicateGroups, groups_b: DuplicateGroups,
    a: &mut Tree, b: &mut Tree, omissions: &mut SyncOmissions,
    opts: BisyncOptions, cancel: &AtomicBool,
) -> Result<(Vec<Conflict>, Vec<Conflict>), (String, String)> {
    let rels: BTreeSet<_> = groups_a.keys().chain(groups_b.keys()).cloned().collect();
    let mut repairs = Vec::new();
    let mut conflicts = Vec::new();
    for rel in rels {
        if omissions.protects(&rel) { continue; }
        // A singleton filtered out on the other side must not be pulled back
        // into the repair by a fresh name lookup (hidden/size/age/ignore rules).
        let excluded = match (|| {
            Ok::<_, std::io::Error>(
                (!groups_a.contains_key(&rel) && !a.contains_key(&rel)
                    && !super::duplicate_observation::metadata(endpoints.a, &join(endpoints.root_a, &rel))?.is_empty())
                || (!groups_b.contains_key(&rel) && !b.contains_key(&rel)
                    && !super::duplicate_observation::metadata(endpoints.b, &join(endpoints.root_b, &rel))?.is_empty())
            )
        })() {
            Ok(excluded) => excluded,
            Err(error) => {
                let kind = crate::vfs::omission_reason(&error).map(OmissionKind::from).unwrap_or(OmissionKind::Unreadable);
                omissions.record_kind(&rel, kind, true);
                a.remove(&rel); b.remove(&rel);
                continue;
            }
        };
        if excluded {
            omissions.record(&rel, false);
            a.remove(&rel);
            b.remove(&rel);
            continue;
        }
        let variants = match (|| {
            Ok::<_, std::io::Error>(DuplicateConflict {
                a: observe(endpoints.a, &join(endpoints.root_a, &rel),
                    groups_a.get(&rel).map(Vec::as_slice), cancel)?,
                b: observe(endpoints.b, &join(endpoints.root_b, &rel),
                    groups_b.get(&rel).map(Vec::as_slice), cancel)?,
            })
        })() {
            Ok(variants) => variants,
            Err(error) => {
                let kind = crate::vfs::omission_reason(&error).map(OmissionKind::from).unwrap_or(OmissionKind::Unreadable);
                omissions.record_kind(&rel, kind, true);
                a.remove(&rel); b.remove(&rel);
                continue;
            }
        };
        let common = variants.common_choice().map(|(a, b)| (a.signature, b.signature));
        let automatic = common.filter(|_| opts.delete != DeletePolicy::NoDelete);
        let conflict = Conflict {
            rel: rel.clone(),
            a: variants.a.first().map(|v| v.signature),
            b: variants.b.first().map(|v| v.signature),
            duplicates: Some(variants),
        };
        if let Some((sa, sb)) = automatic {
            a.insert(rel.clone(), sa);
            b.insert(rel, sb);
            repairs.push(conflict);
        } else {
            a.remove(&rel);
            b.remove(&rel);
            conflicts.push(conflict);
        }
    }
    Ok((repairs, conflicts))
}
