//! Retention by original file, side and replica; all stores use the same buckets.
use super::types::{Versioning, VersioningScheme};
use super::version_listing::Managed;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn selected(entries: &[Managed], versioning: &Versioning) -> BTreeSet<usize> {
    let now = super::versions::now_ms().max(0) as u64 / 1000;
    let mut groups: BTreeMap<(&str, &str, &str, &str, &str), Vec<usize>> = BTreeMap::new();
    for (index, item) in entries.iter().enumerate() {
        let manifest = &item.manifest;
        groups
            .entry((
                &manifest.backend,
                &manifest.root,
                &manifest.replica,
                &manifest.side,
                &manifest.rel,
            ))
            .or_default()
            .push(index);
    }
    let mut delete = BTreeSet::new();
    for mut indices in groups.into_values() {
        indices.sort_by_key(|index| std::cmp::Reverse(entries[*index].entry.preserved_ms));
        let mut buckets = BTreeSet::new();
        for (position, index) in indices.into_iter().enumerate() {
            let stamp = entries[index].entry.preserved_ms.max(0) as u64 / 1000;
            let keep = keep_version(versioning, position, stamp, now, &mut buckets);
            if !keep {
                delete.insert(index);
            }
        }
    }
    delete
}

pub(super) fn keep_version(
    versioning: &Versioning,
    position: usize,
    stamp: u64,
    now: u64,
    buckets: &mut BTreeSet<String>,
) -> bool {
    let age = now.saturating_sub(stamp);
    match versioning.scheme {
        VersioningScheme::Days => {
            versioning.days == 0 || age <= versioning.days.saturating_mul(86400)
        }
        VersioningScheme::Count => versioning.count == 0 || (position as u64) < versioning.count,
        VersioningScheme::Staggered => buckets.insert(if age < 86400 {
            format!("s{stamp}")
        } else if age < 30 * 86400 {
            format!("d{}", stamp / 86400)
        } else {
            format!("w{}", stamp / (7 * 86400))
        }),
        VersioningScheme::Gfs => {
            let bucket = if age < 86400 {
                Some(format!("h{}", stamp / 3600))
            } else if age < 7 * 86400 {
                Some(format!("d{}", stamp / 86400))
            } else if age < 28 * 86400 {
                Some(format!("w{}", stamp / (7 * 86400)))
            } else if age < 365 * 86400 {
                Some(format!("m{}", stamp / (30 * 86400)))
            } else {
                None
            };
            bucket.is_some_and(|bucket| buckets.insert(bucket))
        }
    }
}
