//! `speed_task_drive_*`: files written to and read back from a drive engine
//! over a local folder (the G5 cases 5000 × 4 KiB, 1000 × 1 MiB and
//! 1 × 2 GiB), printing `BENCH drive_<write|read>_<case> files_per_s=<n>
//! mib_per_s=<n>`. The engine runs in this process like on Android; the
//! host/daemon loopback of the desktop drives is not part of the number.
use super::{
    BackendRoot, MountCachePolicy, MountEngine, MountId, MountMode, MountRuntimeConfig,
    OpenDisposition, OpenFileOptions,
};
use crate::vfs::LocalBackend;
use std::sync::Arc;
use std::time::Instant;

const BLOCK: usize = 1024 * 1024;

fn report(case: &str, files: usize, bytes: u64, started: Instant) {
    let seconds = started.elapsed().as_secs_f64().max(1e-9);
    println!(
        "BENCH {case} files_per_s={:.1} mib_per_s={:.1}",
        files as f64 / seconds,
        bytes as f64 / (1024.0 * 1024.0) / seconds
    );
}

#[test]
#[ignore = "benchmark: cargo test speed_task_drive -- --ignored --nocapture"]
fn speed_task_drive_write_and_read_throughput() {
    for (case, files, size) in [
        ("5000x4KiB", 5000usize, 4096u64),
        ("1000x1MiB", 1000, 1 << 20),
        ("1x2GiB", 1, 2 << 30),
    ] {
        let remote = tempfile::tempdir().unwrap();
        let spool = tempfile::tempdir().unwrap();
        let root = remote.path().to_string_lossy().into_owned();
        let config =
            MountRuntimeConfig::new(MountId::parse("speed-task").unwrap(), MountMode::ReadWrite)
                .with_cache_policy(MountCachePolicy::new(0).unwrap());
        let engine = MountEngine::open_rooted(
            config,
            BackendRoot::parse(&root).unwrap(),
            Arc::new(LocalBackend::new(&root)),
            spool.path(),
        )
        .unwrap();
        let block = vec![0x6bu8; BLOCK];
        let started = Instant::now();
        for index in 0..files {
            let handle = engine
                .open_file(
                    &format!("\\f{index}.bin"),
                    OpenFileOptions {
                        writable: true,
                        disposition: OpenDisposition::CreateNew,
                    },
                )
                .unwrap();
            let mut offset = 0u64;
            while offset < size {
                let part = (size - offset).min(BLOCK as u64) as usize;
                engine.write(handle, offset, &block[..part]).unwrap();
                offset += part as u64;
            }
            engine.flush(handle).unwrap();
            engine.close(handle).unwrap();
        }
        report(
            &format!("drive_write_{case}"),
            files,
            files as u64 * size,
            started,
        );
        let mut buffer = vec![0u8; BLOCK];
        let started = Instant::now();
        for index in 0..files {
            let handle = engine
                .open_file(
                    &format!("\\f{index}.bin"),
                    OpenFileOptions {
                        writable: false,
                        disposition: OpenDisposition::OpenExisting,
                    },
                )
                .unwrap();
            let mut offset = 0u64;
            loop {
                let read = engine.read(handle, offset, &mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                offset += read as u64;
            }
            assert_eq!(offset, size);
            engine.close(handle).unwrap();
        }
        report(
            &format!("drive_read_{case}"),
            files,
            files as u64 * size,
            started,
        );
    }
}

/// G4 ML1/ML4: the time `close` takes with the background write-back
/// (`drive_close_<case>`), until every upload is confirmed
/// (`drive_uploaded_<case>`), and the first byte of a streamed read
/// (`BENCH drive_first_byte_<case> ms=<n>`).
#[test]
#[ignore = "benchmark: cargo test speed_task_drive -- --ignored --nocapture"]
fn speed_task_drive_write_back_and_first_byte() {
    for (case, files, size) in [
        ("5000x4KiB", 5000usize, 4096u64),
        ("1000x1MiB", 1000, 1 << 20),
        ("1x2GiB", 1, 2 << 30),
    ] {
        let remote = tempfile::tempdir().unwrap();
        let spool = tempfile::tempdir().unwrap();
        let root = remote.path().to_string_lossy().into_owned();
        let config =
            MountRuntimeConfig::new(MountId::parse("speed-task").unwrap(), MountMode::ReadWrite)
                .with_cache_policy(MountCachePolicy::new(0).unwrap());
        let engine = MountEngine::open_rooted(
            config,
            BackendRoot::parse(&root).unwrap(),
            Arc::new(LocalBackend::new(&root)),
            spool.path(),
        )
        .unwrap();
        let engine = Arc::new(engine);
        engine.start_write_back();
        let block = vec![0x6bu8; BLOCK];
        let started = Instant::now();
        for index in 0..files {
            let handle = engine
                .open_file(
                    &format!("\\f{index}.bin"),
                    OpenFileOptions {
                        writable: true,
                        disposition: OpenDisposition::CreateNew,
                    },
                )
                .unwrap();
            let mut offset = 0u64;
            while offset < size {
                let part = (size - offset).min(BLOCK as u64) as usize;
                engine.write(handle, offset, &block[..part]).unwrap();
                offset += part as u64;
            }
            engine.flush_on_close(handle).unwrap();
            engine.close(handle).unwrap();
        }
        let bytes = files as u64 * size;
        report(&format!("drive_close_{case}"), files, bytes, started);
        engine.finish_write_back();
        report(&format!("drive_uploaded_{case}"), files, bytes, started);
        assert!(engine.dirty_entries().unwrap().is_empty());
        let mut first = vec![0u8; 64 * 1024];
        let started = Instant::now();
        let handle = engine
            .open_file(
                "\\f0.bin",
                OpenFileOptions {
                    writable: false,
                    disposition: OpenDisposition::OpenExisting,
                },
            )
            .unwrap();
        let read = engine.read(handle, 0, &mut first).unwrap();
        assert!(read > 0);
        println!(
            "BENCH drive_first_byte_{case} ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0
        );
        engine.close(handle).unwrap();
    }
}
