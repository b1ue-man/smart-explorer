//! A content-bound selection for the storing device's trash. Report paths
//! are backend paths, never paths of the viewing device. Ambiguous names and
//! provider/agent MD5 groups cannot authorise a SHA-256 recycle request.
use std::collections::{HashMap, HashSet};

use crate::analytics::{DuplicateGroup, HashAlgorithm};
use crate::vfs::RecycleExpectation;

pub(crate) struct RecyclePlan {
    pub(crate) targets: Vec<(String, RecycleExpectation)>,
    pub(crate) bytes: u64,
    pub(crate) skipped: usize,
    pub(crate) kept: usize,
}

pub(crate) fn recycle_plan(groups: &[DuplicateGroup], selected: &HashSet<String>) -> RecyclePlan {
    let mut plan = RecyclePlan {
        targets: Vec::new(),
        bytes: 0,
        skipped: 0,
        kept: 0,
    };
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for item in groups.iter().flat_map(|group| &group.items) {
        *counts.entry(&item.path).or_default() += 1;
    }
    let mut covered = HashSet::new();
    for group in groups {
        if group.items.len() < 2
            || group.hash.algorithm != HashAlgorithm::Sha256
            || group.hash.hex.len() != 64
            || !group.hash.hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            continue;
        }
        let chosen: Vec<_> = group
            .items
            .iter()
            .filter(|item| {
                selected.contains(&item.path) && counts.get(item.path.as_str()) == Some(&1)
            })
            .collect();
        let keep_first = chosen.len() == group.items.len();
        for (index, item) in chosen.into_iter().enumerate() {
            covered.insert(item.path.clone());
            if keep_first && index == 0 {
                plan.kept += 1;
                continue;
            }
            plan.targets.push((
                item.path.clone(),
                RecycleExpectation {
                    size: group.size,
                    sha256: Some(group.hash.hex.to_ascii_lowercase()),
                },
            ));
            plan.bytes = plan.bytes.saturating_add(group.size);
        }
    }
    plan.skipped = selected
        .iter()
        .filter(|path| !covered.contains(*path))
        .count();
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analytics::{ContentHash, DuplicateEvidence, ReclaimItem};

    fn group(paths: &[&str]) -> DuplicateGroup {
        DuplicateGroup {
            size: 20,
            reclaimable: 20,
            hash: ContentHash {
                algorithm: HashAlgorithm::Sha256,
                hex: "AB".repeat(32),
            },
            evidence: DuplicateEvidence::LocalSha256,
            items: paths
                .iter()
                .map(|path| ReclaimItem::new((*path).into(), (*path).into(), 20, 0, false))
                .collect(),
        }
    }

    #[test]
    fn review_task_remote_recycle_rejects_unverified_and_ambiguous_copies() {
        let mut unverified = group(&["/md5-a", "/md5-b"]);
        unverified.hash.algorithm = HashAlgorithm::Md5;
        let mut invalid_hash = group(&["/bad-a", "/bad-b"]);
        invalid_hash.hash.hex = "bad".into();
        let groups = [
            group(&["/keep", "/copy"]),
            group(&["/same", "/same"]),
            unverified,
            invalid_hash,
        ];
        let selected = ["/copy", "/same", "/md5-b", "/bad-b"]
            .map(String::from)
            .into_iter()
            .collect();
        let plan = recycle_plan(&groups, &selected);
        assert_eq!(
            (plan.targets.len(), plan.skipped, plan.kept, plan.bytes),
            (1, 3, 0, 20)
        );
        assert_eq!(plan.targets[0].0, "/copy");
        assert_eq!(
            plan.targets[0].1.sha256.as_deref(),
            Some("ab".repeat(32).as_str())
        );
        let plan = recycle_plan(&[group(&["/one"])], &["/one".into()].into_iter().collect());
        assert!(plan.targets.is_empty());
        assert_eq!(plan.skipped, 1);
    }
}
