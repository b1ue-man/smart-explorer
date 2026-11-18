//! Occupied bytes over the Share wire: the analysis capability and request
//! field, and duplicate copies with and without their occupancy.
use super::*;
use crate::share::wire::{FsHostFeatures, FsStorageAnalysis};

#[test]
fn belegt_task_share_wire_negotiates_allocation_and_old_hosts_stay_unknown() {
    assert!(FsHostFeatures::host().analysis_allocated_v1);
    assert!(FsHostFeatures::host()
        .names()
        .contains(&"analysis_allocated_v1"));
    let old_host: FsHostFeatures = serde_json::from_str(r#"{"watch_v1":true}"#).expect("old");
    assert!(!old_host.analysis_allocated_v1);
    let plain = FsStorageAnalysis {
        path: "/A".into(),
        ..Default::default()
    };
    let plain = serde_json::to_string(&plain).expect("json");
    assert!(!plain.contains("allocated"), "{plain}");
    let older: FsStorageAnalysis = serde_json::from_str(r#"{"path":"/A"}"#).expect("older");
    assert!(!older.allocated);

    // A host with the field: each copy's occupancy, the placeholder frees 0.
    let group = FsDuplicateGroup {
        sha256: "ab".repeat(32),
        size: 1000,
        files: vec![
            FsDuplicateFile {
                path: "/full".into(),
                mtime_ms: 2,
                allocated: Some(4096),
            },
            FsDuplicateFile {
                path: "/placeholder".into(),
                mtime_ms: 1,
                allocated: Some(0),
            },
        ],
        more: false,
    };
    let json = serde_json::to_string(&group).expect("json");
    let back: FsDuplicateGroup = serde_json::from_str(&json).expect("back");
    assert_eq!(back.clone().into_group().reclaimable, 0);
    // A host before it: no occupancy, every copy occupies the group size.
    let old = json.replace(r#","allocated":4096"#, "");
    let old = old.replace(r#","allocated":0"#, "");
    assert!(!old.contains("allocated"), "{old}");
    let old: FsDuplicateGroup = serde_json::from_str(&old).expect("old group");
    let old = old.into_group();
    assert!(old.items.iter().all(|item| item.allocated == 1000));
    assert_eq!(old.reclaimable, 1000);
}
