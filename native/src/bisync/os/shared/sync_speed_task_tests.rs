//! `speed_task_sync_*`: a complete first run (scan, plan, apply, checkpoint,
//! state) between two local folders for the G5 cases 5000 × 4 KiB (50
//! folders), 1000 × 1 MiB (10 folders) and 1 × 2 GiB, printing
//! `BENCH sync_local_<case> files_per_s=<n> mib_per_s=<n>`, and a second,
//! unchanged run (`sync_local_rescan_<case>`). Since G4 MS14 the first run
//! copies new files while the scan still runs; the case with 1000 folders
//! shows that overlap.
use super::checkpoint_review_tests::Fixture;
use super::*;
use std::io::Write;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

fn fill(root: &std::path::Path, files: usize, folders: usize, size: usize) {
    let block: Vec<u8> = (0..size.min(1 << 20)).map(|n| (n % 251) as u8).collect();
    for index in 0..files {
        let folder = root.join(format!("d{}", index % folders));
        std::fs::create_dir_all(&folder).unwrap();
        let mut file = std::fs::File::create(folder.join(format!("f{index}.bin"))).unwrap();
        let mut left = size;
        while left > 0 {
            let part = left.min(block.len());
            file.write_all(&block[..part]).unwrap();
            left -= part;
        }
    }
}

fn report(case: &str, files: usize, bytes: usize, started: Instant) {
    let seconds = started.elapsed().as_secs_f64().max(1e-9);
    println!(
        "BENCH {case} files_per_s={:.1} mib_per_s={:.1}",
        files as f64 / seconds,
        bytes as f64 / (1024.0 * 1024.0) / seconds
    );
}

#[test]
#[ignore = "benchmark: cargo test speed_task_sync -- --ignored --nocapture"]
fn speed_task_sync_local_first_run_throughput() {
    for (case, files, folders, size) in [
        ("5000x4KiB", 5000usize, 50usize, 4096usize),
        // G4 MS14: many folders, so copies overlap a longer scan.
        ("5000x4KiB_in_1000_folders", 5000, 1000, 4096),
        ("1000x1MiB", 1000, 10, 1 << 20),
        ("1x2GiB", 1, 1, 2 << 30),
    ] {
        let fixture = Fixture::new();
        fill(&fixture.root.join("a"), files, folders, size);
        let ignore = empty_globset();
        let filter = WalkFilter::basic(true, &ignore);
        let cancel = AtomicBool::new(false);
        let store = fixture.root.join("index.sqlite");
        let opts = BisyncOptions::default();
        let started = Instant::now();
        let first =
            orchestration::run_with_store_path(fixture.endpoints(), opts, &cancel, &filter, &store);
        report(&format!("sync_local_{case}"), files, files * size, started);
        assert!(first.errors.is_empty(), "{:?}", first.errors);
        assert_eq!(first.stats.a_to_b as usize, files);
        let started = Instant::now();
        let again =
            orchestration::run_with_store_path(fixture.endpoints(), opts, &cancel, &filter, &store);
        report(&format!("sync_local_rescan_{case}"), files, 0, started);
        assert_eq!(again.stats.a_to_b + again.stats.b_to_a, 0);
    }
}
