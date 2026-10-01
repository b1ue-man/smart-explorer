use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use sha2::Digest as _;

use super::finder::{
    find_duplicates_in, DuplicateReport, FinderLimits, Guard, MAX_CANDIDATE_TEXT_BYTES,
};
use super::stage::ReclaimPhase;
use super::types::{
    DuplicateEvidence, HashAlgorithm, ReclaimProgress, ReclaimReport, ReclaimResultCounts,
};
use super::util::hex_lower;
use crate::analytics::ProtectedOmission;
use crate::apptrash::ProtectedAreas;

fn write(path: &Path, content: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
    std::fs::write(path, content).expect("write");
}

fn search(
    root: &Path,
    threads: usize,
    text: u64,
    protected: &ProtectedAreas,
    guard: Option<Guard<'_>>,
) -> DuplicateReport {
    let limits = FinderLimits {
        min_bytes: 1,
        candidate_text_bytes: text,
        threads,
    };
    find_duplicates_in(root, &ReclaimProgress::default(), limits, protected, guard)
}

fn sha256_hex(content: &[u8]) -> String {
    hex_lower(&sha2::Sha256::digest(content))
}

fn shape(report: &DuplicateReport) -> Vec<(String, Vec<String>)> {
    report
        .groups
        .iter()
        .map(|group| {
            let mut paths: Vec<String> = group.items.iter().map(|item| item.path.clone()).collect();
            paths.sort();
            (group.hash.hex.clone(), paths)
        })
        .collect()
}

/// More than 200 candidates; the duplicates are the smallest files, which the
/// desktop's 200-largest cap would never compare.
fn crowded_tree() -> (tempfile::TempDir, PathBuf, Vec<u8>, Vec<u8>) {
    let fixture = tempfile::tempdir().expect("fixture");
    let root = std::fs::canonicalize(fixture.path()).expect("canonical");
    for index in 0..230u32 {
        let size = 1000 + index as usize;
        write(
            &root.join(format!("unique/{index:03}.bin")),
            &vec![index as u8; size],
        );
    }
    let small = b"duplicate!".to_vec();
    write(&root.join("small-a.txt"), &small);
    write(&root.join("small-b.txt"), &small);
    write(&root.join("deep/er/small-c.txt"), &small);
    write(&root.join("middle-a.bin"), b"aa1111zz");
    write(&root.join("middle-b.bin"), b"aa2222zz");
    let large: Vec<u8> = (0..200 * 1024u32)
        .map(|value| (value % 251) as u8)
        .collect();
    write(&root.join("large-a.bin"), &large);
    write(&root.join("copies/large-b.bin"), &large);
    // Details below build/cache folders are never offered, as on the desktop.
    write(&root.join("cache/small-d.txt"), &small);
    (fixture, root, small, large)
}

#[test]
fn android_background_task_duplicates_compare_every_candidate_in_parallel() {
    let (_fixture, root, small, large) = crowded_tree();
    let none = ProtectedAreas::default();
    let parallel = search(&root, 4, MAX_CANDIDATE_TEXT_BYTES, &none, None);
    let summary = &parallel.summary;
    assert_eq!(summary.candidates, 237, "{summary:?}");
    assert!(summary.candidates > 200);
    assert_eq!(summary.compared, 7);
    assert_eq!(summary.files, 238);
    assert_eq!(summary.groups, 2);
    assert!(summary.errors.is_empty(), "{:?}", summary.errors);
    assert!(summary.limits.is_empty());
    assert!(parallel.root_error.is_none());

    let largest = &parallel.groups[0];
    assert_eq!(largest.size, large.len() as u64);
    assert_eq!(largest.hash.hex, sha256_hex(&large));
    assert_eq!(largest.hash.algorithm, HashAlgorithm::Sha256);
    assert_eq!(largest.evidence, DuplicateEvidence::LocalSha256);
    let trio = &parallel.groups[1];
    assert_eq!(trio.items.len(), 3);
    assert_eq!(trio.hash.hex, sha256_hex(&small));
    assert_eq!(trio.reclaimable, 2 * small.len() as u64);
    assert!(trio
        .items
        .iter()
        .all(|item| !item.path.ends_with("small-d.txt")));

    let serial = search(&root, 1, MAX_CANDIDATE_TEXT_BYTES, &none, None);
    assert_eq!(shape(&serial), shape(&parallel));
    assert_eq!(serial.summary, parallel.summary);
}

#[test]
fn android_background_task_duplicates_report_the_candidate_budget() {
    let (_fixture, root, _, _) = crowded_tree();
    let one_path = root.join("unique/000.bin").as_os_str().len() as u64;
    let report = search(&root, 2, 3 * one_path, &ProtectedAreas::default(), None);
    let summary = report.summary.view();
    assert_eq!(summary.candidates, 237);
    let limit = summary.limit.expect("the spent budget is visible");
    assert!(limit.contains("Kandidatenspeicher"), "{limit}");
    assert!(report.summary.compared <= 3);
}

#[test]
fn android_background_task_duplicates_treat_protected_areas_as_omissions() {
    let fixture = tempfile::tempdir().expect("fixture");
    let root = std::fs::canonicalize(fixture.path()).expect("canonical");
    write(&root.join("Android/data/own/a.bin"), b"same bytes");
    write(&root.join("Android/data/own/b.bin"), b"same bytes");
    write(&root.join("Android/data/foreign/x.bin"), b"same bytes");
    std::fs::create_dir_all(root.join("Android/obb")).expect("obb");
    let volumes = [root.clone()];
    let deny = |path: &Path| -> std::io::Result<()> {
        if path.ends_with("foreign") {
            Err(std::io::ErrorKind::PermissionDenied.into())
        } else {
            Ok(())
        }
    };
    let areas = ProtectedAreas::for_walk_in(&root, &volumes);
    let report = search(&root, 2, MAX_CANDIDATE_TEXT_BYTES, &areas, Some(&deny));
    assert!(
        report.summary.errors.is_empty(),
        "{:?}",
        report.summary.errors
    );
    assert_eq!(report.summary.groups, 1);
    let area = |name: &str, entries| ProtectedOmission {
        area: root.join(name).to_string_lossy().into_owned(),
        entries,
    };
    assert_eq!(
        report.summary.protected,
        [area("Android/data", 1), area("Android/obb", 0)]
    );
    let view = report.summary.view();
    assert_eq!(view.protected_count, 1);
    assert!(view.protected_text.contains("Android/data"));

    let gone = root.join("Android/data/gone");
    let areas = ProtectedAreas::for_walk_in(&gone, &volumes);
    let report = search(&gone, 2, MAX_CANDIDATE_TEXT_BYTES, &areas, None);
    assert!(report.root_error.is_none());
    assert_eq!(report.summary.protected, [area("Android/data", 1)]);

    let missing = search(
        &root.join("missing"),
        2,
        MAX_CANDIDATE_TEXT_BYTES,
        &areas,
        None,
    );
    assert!(
        missing.root_error.is_some(),
        "outside protected areas a missing root fails"
    );
}

#[test]
fn android_background_task_duplicates_report_phases_and_cancel() {
    let (_fixture, root, _, _) = crowded_tree();
    let progress = ReclaimProgress::default();
    assert_eq!(progress.stage.phase(), ReclaimPhase::Walking);
    assert_eq!(progress.status_line(), "0 Ordner");
    let limits = FinderLimits {
        min_bytes: 1,
        candidate_text_bytes: MAX_CANDIDATE_TEXT_BYTES,
        threads: 2,
    };
    let report = find_duplicates_in(&root, &progress, limits, &ProtectedAreas::default(), None);
    assert_eq!(report.summary.groups, 2);
    assert_eq!(progress.stage.phase(), ReclaimPhase::Grouping);
    assert_eq!(progress.fingerprinted.load(Ordering::Relaxed), 7);
    assert_eq!(progress.hashed.load(Ordering::Relaxed), 5);
    assert_eq!(progress.candidates.load(Ordering::Relaxed), 5);

    let canceled = ReclaimProgress::default();
    canceled.cancel.store(true, Ordering::Relaxed);
    let report = find_duplicates_in(&root, &canceled, limits, &ProtectedAreas::default(), None);
    assert!(report.groups.is_empty());
    assert_eq!(canceled.hashed.load(Ordering::Relaxed), 0);
}

#[test]
fn android_background_task_remote_reports_name_their_caps() {
    let report = ReclaimReport {
        files: 900,
        bytes: 4096,
        scan_limit: Some("bounded entry count limit".into()),
        duplicate_candidates: 300,
        duplicate_candidates_retained: 200,
        result_counts: ReclaimResultCounts {
            duplicate_groups: 250,
            ..Default::default()
        },
        errors: vec!["/x: denied".into()],
        suppressed_errors: 2,
        ..Default::default()
    };
    let report = DuplicateReport::from_reclaim(report);
    let view = report.summary.view();
    assert_eq!((view.files, view.candidates, view.groups), (900, 300, 0));
    assert_eq!(view.error_count, 3);
    assert_eq!(view.error_text, "/x: denied\n… 2 weitere");
    let limit = view.limit.expect("limits");
    assert_eq!(limit.lines().count(), 3, "{limit}");
    assert!(limit.contains("200 größten von 300"), "{limit}");
    let json = serde_json::to_value(&view).expect("json");
    for key in [
        "files",
        "bytes",
        "candidates",
        "compared",
        "groups",
        "protectedCount",
        "protectedText",
        "errorCount",
        "errorText",
        "limit",
    ] {
        assert!(json.get(key).is_some(), "{key}");
    }
}
