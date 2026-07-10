use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

use super::backend::RemoteCandidate;
use super::duplicates::DuplicateAnalysis;
use super::retention::{compare_group, retain_best};
use super::types::{
    ContentHash, DuplicateEvidence, DuplicateGroup, HashAlgorithm, ReclaimItem, ReclaimProgress,
};

pub(super) fn remote_duplicate_groups(
    files: Vec<RemoteCandidate>,
    progress: &ReclaimProgress,
    limit: usize,
) -> DuplicateAnalysis {
    let mut retained = Vec::with_capacity(limit.min(files.len()));
    for file in files {
        retain_best(&mut retained, file, limit, |left, right| {
            right
                .item
                .size
                .cmp(&left.item.size)
                .then_with(|| left.item.path.cmp(&right.item.path))
        });
    }
    retained.sort_by(|left, right| {
        right
            .item
            .size
            .cmp(&left.item.size)
            .then_with(|| left.item.path.cmp(&right.item.path))
    });
    let mut by_key: BTreeMap<(u64, String, DuplicateEvidence), Vec<ReclaimItem>> = BTreeMap::new();
    for file in retained {
        by_key
            .entry((file.item.size, file.md5, file.evidence))
            .or_default()
            .push(file.item);
    }
    let mut groups = Vec::new();
    let mut total_groups = 0u64;
    for ((size, md5, evidence), mut items) in by_key {
        if items.len() < 2 {
            continue;
        }
        items.sort_by(|left, right| {
            right
                .mtime_ms
                .cmp(&left.mtime_ms)
                .then_with(|| left.path.cmp(&right.path))
        });
        let reclaimable = size.saturating_mul(items.len().saturating_sub(1) as u64);
        progress
            .candidates
            .fetch_add(items.len() as u64, Ordering::Relaxed);
        total_groups = total_groups.saturating_add(1);
        retain_best(
            &mut groups,
            DuplicateGroup {
                hash: ContentHash {
                    algorithm: HashAlgorithm::Md5,
                    hex: md5,
                },
                evidence,
                size,
                reclaimable,
                items,
            },
            limit,
            compare_group,
        );
    }
    groups.sort_by(compare_group);
    DuplicateAnalysis {
        groups,
        total_groups,
    }
}
