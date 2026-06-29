use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::duplicates::bytes_equal;
use super::types::{ReclaimConfidence, ReclaimItem, ReclaimProgress, ReclaimReport};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReclaimTrashPlan {
    pub delete_paths: Vec<String>,
    pub verified_duplicate_paths: Vec<String>,
    pub skipped_paths: Vec<String>,
    pub risky_paths: Vec<String>,
    pub estimated_bytes: u64,
}

pub fn prepare_reclaim_trash_plan(
    report: &ReclaimReport,
    selected_paths: &[String],
) -> ReclaimTrashPlan {
    if report.is_remote {
        return ReclaimTrashPlan {
            skipped_paths: selected_paths.to_vec(),
            ..ReclaimTrashPlan::default()
        };
    }

    let selected: HashSet<&str> = selected_paths.iter().map(String::as_str).collect();
    let by_path = item_map(report);
    let mut duplicate_paths = HashSet::new();
    let progress = ReclaimProgress::default();
    let mut plan = ReclaimTrashPlan::default();

    for group in &report.duplicate_groups {
        let group_selected: Vec<&ReclaimItem> = group
            .items
            .iter()
            .filter(|item| selected.contains(item.path.as_str()))
            .collect();
        if group_selected.is_empty() {
            continue;
        }
        let keeper = group
            .items
            .iter()
            .find(|item| !selected.contains(item.path.as_str()))
            .unwrap_or(&group.items[0]);
        for item in group_selected {
            duplicate_paths.insert(item.path.as_str());
            if item.path == keeper.path {
                plan.skipped_paths.push(item.path.clone());
                continue;
            }
            let a = native_path(&keeper.path);
            let b = native_path(&item.path);
            match bytes_equal(&a, &b, &progress) {
                Ok(Some(true)) => {
                    plan.verified_duplicate_paths.push(item.path.clone());
                    push_delete(&mut plan, item);
                }
                Ok(Some(false)) | Ok(None) => plan.skipped_paths.push(item.path.clone()),
                Err(_) => plan.skipped_paths.push(item.path.clone()),
            }
        }
    }

    for path in selected_paths {
        if duplicate_paths.contains(path.as_str()) {
            continue;
        }
        if let Some(item) = by_path.get(path.as_str()) {
            push_delete(&mut plan, item);
        }
    }

    plan.delete_paths = dedupe_nested_paths(&plan.delete_paths);
    plan.skipped_paths.sort();
    plan.skipped_paths.dedup();
    plan.risky_paths.sort();
    plan.risky_paths.dedup();
    plan.verified_duplicate_paths.sort();
    plan.verified_duplicate_paths.dedup();
    plan
}

fn item_map(report: &ReclaimReport) -> HashMap<&str, &ReclaimItem> {
    let mut out = HashMap::new();
    for item in report
        .large_files
        .iter()
        .chain(report.stale_files.iter())
        .chain(report.empty_files.iter())
        .chain(report.empty_dirs.iter())
        .chain(report.cleanup.iter())
        .chain(report.duplicate_groups.iter().flat_map(|g| g.items.iter()))
    {
        out.entry(item.path.as_str()).or_insert(item);
    }
    out
}

fn push_delete(plan: &mut ReclaimTrashPlan, item: &ReclaimItem) {
    if item.confidence.needs_warning() || matches!(item.confidence, ReclaimConfidence::HashMatch) {
        plan.risky_paths.push(item.path.clone());
    }
    plan.estimated_bytes = plan.estimated_bytes.saturating_add(item.size);
    plan.delete_paths.push(item.path.clone());
}

fn native_path(path: &str) -> PathBuf {
    PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR))
}

pub(crate) fn dedupe_nested_paths(paths: &[String]) -> Vec<String> {
    let mut sorted = paths.to_vec();
    sorted.sort_by_key(|p| p.matches('/').count());
    let mut out: Vec<String> = Vec::new();
    'next: for p in sorted {
        for kept in &out {
            if p == *kept || p.starts_with(&format!("{}/", kept.trim_end_matches('/'))) {
                continue 'next;
            }
        }
        out.push(p);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analytics::{
        ContentHash, DuplicateEvidence, DuplicateGroup, HashAlgorithm, ReclaimReport,
    };

    #[test]
    fn changed_duplicate_is_skipped_before_trash() {
        let base = std::env::temp_dir().join(format!("se_verify_changed_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let keeper = base.join("keeper.bin");
        let copy = base.join("copy.bin");
        std::fs::write(&keeper, b"same").unwrap();
        std::fs::write(&copy, b"same").unwrap();
        let keeper_s = keeper.to_string_lossy().replace('\\', "/");
        let copy_s = copy.to_string_lossy().replace('\\', "/");
        let mut keep_item = ReclaimItem::new(keeper_s.clone(), "keeper.bin".into(), 4, 2, false);
        keep_item.confidence = ReclaimConfidence::HashMatch;
        let mut copy_item = ReclaimItem::new(copy_s.clone(), "copy.bin".into(), 4, 1, false);
        copy_item.confidence = ReclaimConfidence::HashMatch;
        let report = ReclaimReport {
            duplicate_groups: vec![DuplicateGroup {
                hash: ContentHash {
                    algorithm: HashAlgorithm::Sha256,
                    hex: "abc".into(),
                },
                evidence: DuplicateEvidence::LocalSha256,
                size: 4,
                reclaimable: 4,
                items: vec![keep_item, copy_item],
            }],
            ..ReclaimReport::default()
        };
        std::fs::write(&copy, b"diff").unwrap();
        let plan = prepare_reclaim_trash_plan(&report, &[copy_s.clone()]);
        assert!(plan.delete_paths.is_empty());
        assert_eq!(plan.skipped_paths, vec![copy_s]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn duplicate_copy_is_verified_before_trash() {
        let base = std::env::temp_dir().join(format!("se_verify_same_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let keeper = base.join("keeper.bin");
        let copy = base.join("copy.bin");
        std::fs::write(&keeper, b"same").unwrap();
        std::fs::write(&copy, b"same").unwrap();
        let keeper_s = keeper.to_string_lossy().replace('\\', "/");
        let copy_s = copy.to_string_lossy().replace('\\', "/");
        let mut keep_item = ReclaimItem::new(keeper_s.clone(), "keeper.bin".into(), 4, 2, false);
        keep_item.confidence = ReclaimConfidence::HashMatch;
        let mut copy_item = ReclaimItem::new(copy_s.clone(), "copy.bin".into(), 4, 1, false);
        copy_item.confidence = ReclaimConfidence::HashMatch;
        let report = ReclaimReport {
            duplicate_groups: vec![DuplicateGroup {
                hash: ContentHash {
                    algorithm: HashAlgorithm::Sha256,
                    hex: "abc".into(),
                },
                evidence: DuplicateEvidence::LocalSha256,
                size: 4,
                reclaimable: 4,
                items: vec![keep_item, copy_item],
            }],
            ..ReclaimReport::default()
        };
        let plan = prepare_reclaim_trash_plan(&report, &[copy_s.clone()]);
        assert_eq!(plan.delete_paths, vec![copy_s.clone()]);
        assert_eq!(plan.verified_duplicate_paths, vec![copy_s]);
        assert!(plan.skipped_paths.is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn nested_selection_is_deduped() {
        let paths = vec![
            "C:/x/cache".to_string(),
            "C:/x/cache/a.log".to_string(),
            "C:/x/other".to_string(),
        ];
        assert_eq!(
            dedupe_nested_paths(&paths),
            vec!["C:/x/cache".to_string(), "C:/x/other".to_string()]
        );
    }
}
