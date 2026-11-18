//! What a cleanup frees is the occupied space: duplicates, the quick
//! selection, the trash plan and the local scan's folder and file items.
use super::types::{
    ContentHash, DuplicateEvidence, DuplicateGroup, HashAlgorithm, ReclaimConfidence, ReclaimItem,
    ReclaimReport,
};
use std::collections::HashSet;

fn item(path: &str, size: u64, allocated: u64) -> ReclaimItem {
    ReclaimItem::new(path.into(), path.into(), size, 0, false).with_allocated(allocated)
}

fn group(items: Vec<ReclaimItem>) -> DuplicateGroup {
    DuplicateGroup {
        hash: ContentHash {
            algorithm: HashAlgorithm::Sha256,
            hex: "ab".repeat(32),
        },
        evidence: DuplicateEvidence::LocalSha256,
        size: items[0].size,
        reclaimable: DuplicateGroup::reclaimable_of(&items),
        items,
    }
}

#[test]
fn belegt_task_reclaim_counts_occupied_bytes() {
    // Three copies of 5 GB: one hydrated, one compressed, one placeholder.
    let copies = vec![
        item("/full", 5_000_000_000, 5_000_000_000),
        item("/compressed", 5_000_000_000, 1_000_000_000),
        item("/placeholder", 5_000_000_000, 0),
    ];
    let duplicates = group(copies);
    assert_eq!(duplicates.reclaimable, 1_000_000_000);
    assert_eq!(duplicates.duplicate_size(), 10_000_000_000);
    // A copy without figures occupies its size: as before the change.
    let plain = group(vec![item("/a", 20, 20), item("/b", 20, 20)]);
    assert_eq!(plain.reclaimable, 20);

    let mut report = ReclaimReport {
        duplicate_groups: vec![duplicates],
        empty_files: vec![item("/empty", 0, 0)],
        cleanup: vec![
            item("/node_modules", 900, 100)
                .with_reason("node_modules", ReclaimConfidence::ReviewSafe),
            item("/.git", 900, 800).with_reason(".git", ReclaimConfidence::NeverAuto),
        ],
        ..ReclaimReport::default()
    };
    assert_eq!(report.reclaimable_bytes(), 1_000_000_000 + 100);
    report.prune_paths(&["/full".into()]);
    assert_eq!(report.duplicate_groups[0].reclaimable, 0);

    let groups = [group(vec![item("/keep", 20, 4096), item("/copy", 20, 0)])];
    let selected: HashSet<String> = ["/copy".to_string()].into_iter().collect();
    let plan = crate::analytics::recycle_plan(&groups, &selected);
    assert_eq!(plan.targets.len(), 1);
    assert_eq!(
        plan.targets[0].1.size, 20,
        "the content check keeps the size"
    );
    assert_eq!(plan.bytes, 0, "a placeholder copy frees nothing");
}

/// The local scan takes the platform's occupied bytes: a sparse file in a
/// cleanup folder frees far less than its size, and of two hard links to
/// one file only the first carries it and they are no duplicates.
#[cfg(unix)]
#[test]
fn belegt_task_local_reclaim_scan_uses_platform_occupancy() {
    use super::types::{ReclaimOptions, ReclaimProgress};
    let fixture = tempfile::tempdir().expect("temp dir");
    let base = fixture.path();
    std::fs::create_dir_all(base.join("node_modules/pkg")).expect("dirs");
    std::fs::write(base.join("package.json"), "{}").expect("package");
    let sparse = std::fs::File::create(base.join("node_modules/pkg/big.bin")).expect("create");
    sparse.set_len(32 * 1024 * 1024).expect("extend");
    drop(sparse);
    std::fs::write(base.join("a.bin"), vec![7u8; 20_000]).expect("write");
    std::fs::hard_link(base.join("a.bin"), base.join("b.bin")).expect("link");
    let options = ReclaimOptions {
        large_min_bytes: u64::MAX,
        stale_days: 0,
        max_items: 50,
        duplicate_min_bytes: 1,
        partial_fingerprint_bytes: 4,
    };
    let report = super::scan_reclaim(base, &ReclaimProgress::default(), &options);
    let cleanup = report
        .cleanup
        .iter()
        .find(|item| item.name == "node_modules")
        .expect("cleanup folder");
    assert_eq!(cleanup.size, 32 * 1024 * 1024);
    assert!(cleanup.allocated < cleanup.size);
    assert!(report.allocated < report.bytes);
    // Two links of one file are never offered as duplicates.
    assert!(!report.duplicate_groups.iter().any(|group| {
        group.items.iter().any(|item| item.name == "a.bin")
            && group.items.iter().any(|item| item.name == "b.bin")
    }));
}

/// The duplicate result says whether every copy carries its occupancy: a
/// local search does, a remote listing (`from_reclaim`) does not.
#[test]
fn belegt_task_duplicate_result_names_known_allocation() {
    let fixture = tempfile::tempdir().expect("temp dir");
    for name in ["a.bin", "b.bin"] {
        std::fs::write(fixture.path().join(name), vec![9u8; 50_000]).expect("write");
    }
    let progress = super::types::ReclaimProgress::default();
    let local = super::find_duplicates(fixture.path(), &progress, 1);
    assert!(local.allocated_known);
    assert_eq!(local.groups.len(), 1);
    assert!(local.groups[0]
        .items
        .iter()
        .all(|item| item.allocated >= 50_000));
    let remote = super::DuplicateReport::from_reclaim(ReclaimReport::default());
    assert!(!remote.allocated_known);
}
