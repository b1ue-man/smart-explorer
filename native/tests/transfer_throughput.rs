//! Throughput acceptance of the transfer engine against real servers over a
//! shaped link. `native/test-transfer-engine-task.sh` starts the test
//! containers, shapes loopback with `tc netem` and runs these ignored tests
//! with `--ignored --test-threads=1`; the environment names the servers and
//! the shaping:
//!
//! - `SE_TASK_NETEM_RTT_MS`, `SE_TASK_NETEM_RATE_MBIT`: the shaped link.
//! - the servers as described in `transfer_task/support.rs`.
//!
//! Each case measures the time from submitting a job to the first finished
//! file and the goodput of the whole job, prints them and checks bounds
//! derived from the link: the first file within a few round trips, many small
//! files far faster than one at a time (which costs at least four round trips
//! per file: open, write, close, publish), one large file close to the rate.
#[path = "transfer_task/support.rs"]
mod support;

use smart_explorer::transfer::Endpoint;
use smart_explorer::vfs::BackendHandle;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;
use support::{env, ftp, job, remote_folder, run, sftp, slash, unique, Measured};

/// Small files per case: enough that the adaptive concurrency leaves slow
/// start (a few seconds) and the steady rate dominates the result.
const SMALL_FILES: usize = 2_000;
const SMALL_FILE_BYTES: usize = 4 * 1024;
const SMALL_FOLDERS: usize = 20;
/// One large file: several seconds at the shaped rate, so connection setup
/// and slow start are a small part of the measurement.
const LARGE_FILE_BYTES: usize = 64 * 1024 * 1024;
/// Round trips one small file costs when files are transferred one at a time.
const SERIAL_ROUND_TRIPS_PER_FILE: f64 = 4.0;
/// Required speed-up of many small files over one-at-a-time transfers.
const SMALL_FILE_SPEEDUP: f64 = 8.0;
/// Required share of the shaped rate for one large file (SSH/TCP framing and
/// TCP slow start cost the rest).
const LARGE_FILE_SHARE: f64 = 0.7;

struct Link {
    rtt: Duration,
    bytes_per_second: f64,
}

fn link() -> Link {
    let rtt_ms: u64 = env("SE_TASK_NETEM_RTT_MS")
        .parse()
        .expect("SE_TASK_NETEM_RTT_MS is a whole number of milliseconds");
    let rate_mbit: f64 = env("SE_TASK_NETEM_RATE_MBIT")
        .parse()
        .expect("SE_TASK_NETEM_RATE_MBIT is a number");
    Link {
        rtt: Duration::from_millis(rtt_ms),
        bytes_per_second: rate_mbit * 1_000_000.0 / 8.0,
    }
}

fn local_files(name: &str, folders: usize, files: usize, bytes: usize) -> PathBuf {
    let root = std::env::temp_dir().join(unique(name));
    let content = vec![0x5a_u8; bytes];
    for index in 0..files {
        let folder = root.join(format!("d{:02}", index % folders.max(1)));
        std::fs::create_dir_all(&folder).expect("create local folder");
        let mut file =
            std::fs::File::create(folder.join(format!("f{index:05}.bin"))).expect("create file");
        file.write_all(&content).expect("write file");
    }
    root
}

fn report(case: &str, measured: &Measured) {
    let seconds = measured.total.as_secs_f64().max(0.001);
    println!(
        "throughput {case}: first file {:.3} s, {} files, {} bytes in {:.3} s = {:.1} files/s, {:.2} MB/s",
        measured.first_file.as_secs_f64(),
        measured.files,
        measured.bytes,
        seconds,
        measured.files as f64 / seconds,
        measured.bytes as f64 / seconds / 1_000_000.0
    );
}

/// The first file lands within the setup a single file needs (target folder,
/// stage, write, publish: about ten round trips) plus one progress tick.
fn assert_first_file(case: &str, link: &Link, measured: &Measured) {
    let bound = Duration::from_secs(1) + link.rtt * 10;
    assert!(
        measured.first_file <= bound,
        "{case}: first file after {:?}, bound {bound:?}",
        measured.first_file
    );
}

fn assert_small_files(case: &str, link: &Link, measured: &Measured) {
    assert_eq!(
        measured.files, SMALL_FILES as u64,
        "{case}: every file arrives"
    );
    let serial = 1.0 / (link.rtt.as_secs_f64() * SERIAL_ROUND_TRIPS_PER_FILE);
    let rate = measured.files as f64 / measured.total.as_secs_f64().max(0.001);
    assert!(
        rate >= serial * SMALL_FILE_SPEEDUP,
        "{case}: {rate:.1} files/s, one at a time would reach {serial:.1} files/s, required {:.1}",
        serial * SMALL_FILE_SPEEDUP
    );
}

fn assert_large_file(case: &str, link: &Link, measured: &Measured) {
    assert_eq!(
        measured.bytes, LARGE_FILE_BYTES as u64,
        "{case}: every byte arrives"
    );
    let rate = measured.bytes as f64 / measured.total.as_secs_f64().max(0.001);
    let required = link.bytes_per_second * LARGE_FILE_SHARE;
    assert!(
        rate >= required,
        "{case}: {:.2} MB/s, required {:.2} MB/s",
        rate / 1_000_000.0,
        required / 1_000_000.0
    );
}

fn upload_small_files(case: &str, backend: BackendHandle, root: &str) {
    let link = link();
    let source = local_files(case, SMALL_FOLDERS, SMALL_FILES, SMALL_FILE_BYTES);
    let target_dir = remote_folder(&*backend, root, case);
    let measured = run(
        job(
            Endpoint::Local,
            Endpoint::Remote(backend),
            vec![slash(&source)],
            target_dir,
            case,
        ),
        Duration::from_secs(600),
    );
    report(case, &measured);
    let _ = std::fs::remove_dir_all(&source);
    assert_first_file(case, &link, &measured);
    assert_small_files(case, &link, &measured);
}

fn upload_large_file(case: &str, backend: BackendHandle, root: &str) {
    let link = link();
    let source = local_files(case, 1, 1, LARGE_FILE_BYTES);
    let target_dir = remote_folder(&*backend, root, case);
    let measured = run(
        job(
            Endpoint::Local,
            Endpoint::Remote(backend),
            vec![slash(&source)],
            target_dir,
            case,
        ),
        Duration::from_secs(600),
    );
    report(case, &measured);
    let _ = std::fs::remove_dir_all(&source);
    assert_large_file(case, &link, &measured);
}

fn download_large_file(case: &str, backend: BackendHandle, root: &str) {
    let link = link();
    let folder = remote_folder(&*backend, root, case);
    let remote_file = format!("{folder}/large.bin");
    {
        // A plain write: the folder is new and unique, and FTP has no
        // exclusive create.
        let mut writer = backend
            .open_write(&remote_file)
            .expect("create remote fixture");
        let block = vec![0xa5_u8; 1024 * 1024];
        for _ in 0..LARGE_FILE_BYTES / block.len() {
            writer.write_all(&block).expect("write remote fixture");
        }
        writer.flush().expect("commit remote fixture");
    }
    let target = std::env::temp_dir().join(unique(case));
    std::fs::create_dir_all(&target).expect("create local target");
    let measured = run(
        job(
            Endpoint::Remote(backend),
            Endpoint::Local,
            vec![remote_file],
            slash(&target),
            case,
        ),
        Duration::from_secs(600),
    );
    report(case, &measured);
    let _ = std::fs::remove_dir_all(&target);
    assert_large_file(case, &link, &measured);
}

#[test]
#[ignore = "needs the task suite's SFTP container and shaped loopback"]
fn transfer_engine_task_throughput_sftp_small_files_upload() {
    let (backend, root) = sftp();
    upload_small_files("sftp-small-upload", backend, &root);
}

#[test]
#[ignore = "needs the task suite's SFTP container and shaped loopback"]
fn transfer_engine_task_throughput_sftp_large_file_upload() {
    let (backend, root) = sftp();
    upload_large_file("sftp-large-upload", backend, &root);
}

#[test]
#[ignore = "needs the task suite's SFTP container and shaped loopback"]
fn transfer_engine_task_throughput_sftp_large_file_download() {
    let (backend, root) = sftp();
    download_large_file("sftp-large-download", backend, &root);
}

#[test]
#[ignore = "needs the task suite's FTP container and shaped loopback"]
fn transfer_engine_task_throughput_ftp_small_files_upload() {
    let (backend, root) = ftp();
    upload_small_files("ftp-small-upload", backend, &root);
}

#[test]
#[ignore = "needs the task suite's FTP container and shaped loopback"]
fn transfer_engine_task_throughput_ftp_large_file_download() {
    let (backend, root) = ftp();
    download_large_file("ftp-large-download", backend, &root);
}
