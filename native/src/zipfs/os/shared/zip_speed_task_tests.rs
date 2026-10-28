//! `speed_task_zip_*`: extraction of deflated archives with the G5 cases
//! (5000 × 4 KiB, 1000 × 1 MiB, 1 × 2 GiB) through both paths, printing
//! `BENCH zip_<controlled|replacing>_<case> files_per_s=<n> mib_per_s=<n>`,
//! plus the extraction contracts the parallel lanes must keep.
use std::io::Write;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

fn archive(path: &Path, files: usize, size: usize) {
    let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    let block: Vec<u8> = (0..1usize << 20).map(|n| (n % 97) as u8).collect();
    for index in 0..files {
        writer
            .start_file(format!("d{}/f{index}.bin", index % 50), options)
            .unwrap();
        let mut left = size;
        while left > 0 {
            let part = left.min(block.len());
            writer.write_all(&block[..part]).unwrap();
            left -= part;
        }
    }
    writer.finish().unwrap();
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
#[ignore = "benchmark: cargo test speed_task_zip -- --ignored --nocapture"]
fn speed_task_zip_extract_throughput() {
    for (case, files, size) in [
        ("5000x4KiB", 5000usize, 4096usize),
        ("1000x1MiB", 1000, 1 << 20),
        ("1x2GiB", 1, 2 << 30),
    ] {
        let base = tempfile::tempdir().unwrap();
        let zip_path = base.path().join("bench.zip");
        archive(&zip_path, files, size);
        let text = zip_path.to_string_lossy().replace('\\', "/");
        let controlled = base.path().join("controlled");
        std::fs::create_dir_all(&controlled).unwrap();
        let cancel = AtomicBool::new(false);
        let started = Instant::now();
        let done = super::extract_all_controlled(&text, &controlled, &cancel, |_| {}).unwrap();
        report(
            &format!("zip_controlled_{case}"),
            files,
            files * size,
            started,
        );
        assert_eq!(done.files as usize, files);
        std::fs::remove_dir_all(&controlled).unwrap();
        let replacing = base.path().join("replacing");
        let started = Instant::now();
        assert_eq!(super::extract_all(&text, &replacing).unwrap(), files);
        report(
            &format!("zip_replacing_{case}"),
            files,
            files * size,
            started,
        );
    }
}

#[test]
fn speed_task_zip_parallel_extraction_writes_every_entry_and_refuses_existing_files() {
    let base = tempfile::tempdir().unwrap();
    let zip_path = base.path().join("many.zip");
    archive(&zip_path, 300, 5000);
    let text = zip_path.to_string_lossy().replace('\\', "/");
    let out = base.path().join("out");
    std::fs::create_dir_all(out.join("d7")).unwrap();
    std::fs::write(out.join("d7/f7.bin"), b"kept").unwrap();
    let cancel = AtomicBool::new(false);
    let mut last = super::ExtractProgress::default();
    let report = super::extract_all_controlled(&text, &out, &cancel, |progress| {
        last = progress.clone();
    })
    .unwrap();
    assert_eq!(report.files, 299, "{:?}", report.skipped);
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].0, "d7/f7.bin");
    assert_eq!(std::fs::read(out.join("d7/f7.bin")).unwrap(), b"kept");
    assert_eq!(last.files_done, 300);
    assert_eq!(last.bytes_done, 299 * 5000);
    for index in [0usize, 123, 299] {
        let written = std::fs::read(out.join(format!("d{}/f{index}.bin", index % 50))).unwrap();
        assert_eq!(written.len(), 5000);
        assert!(written
            .iter()
            .enumerate()
            .all(|(n, byte)| *byte == (n % 97) as u8));
    }
}

#[test]
fn speed_task_zip_canceled_extraction_leaves_no_partial_file() {
    let base = tempfile::tempdir().unwrap();
    let zip_path = base.path().join("cancel.zip");
    archive(&zip_path, 4, 64 << 20);
    let text = zip_path.to_string_lossy().replace('\\', "/");
    let out = base.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    let cancel = AtomicBool::new(false);
    let report = super::extract_all_controlled(&text, &out, &cancel, |progress| {
        if progress.bytes_done > 0 {
            cancel.store(true, std::sync::atomic::Ordering::Release);
        }
    })
    .unwrap();
    assert!(report.canceled);
    let mut sizes = Vec::new();
    for folder in std::fs::read_dir(&out).unwrap() {
        for file in std::fs::read_dir(folder.unwrap().path()).unwrap() {
            sizes.push(file.unwrap().metadata().unwrap().len());
        }
    }
    assert!(
        sizes.iter().all(|size| *size == 64 << 20),
        "only complete files remain: {sizes:?}"
    );
}
