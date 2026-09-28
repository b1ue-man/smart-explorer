//! Throughput acceptance of the transfer engine against real servers over a
//! shaped link. `native/test-transfer-engine-task.sh` starts the test
//! containers, shapes loopback with `tc netem` and runs these ignored tests
//! with `--ignored --test-threads=1`; the environment names the servers and
//! the shaping:
//!
//! - `SE_TASK_NETEM_RTT_MS`, `SE_TASK_NETEM_RATE_MBIT`: the shaped link.
//! - `SE_TASK_SFTP`: `host:port:user:password:root` of an SFTP server.
//! - `SE_TASK_FTP_URL`: `ftp://user:password@host:port/root`.
//!
//! Each case measures the time from submitting a job to the first finished
//! file and the goodput of the whole job, prints them and checks bounds
//! derived from the link: the first file within a few round trips, many small
//! files far faster than one at a time (which costs at least four round trips
//! per file: open, write, close, publish), one large file close to the rate.
use smart_explorer::transfer::{
    launch_transfer, Endpoint, JobItems, Layout, TransferJob, TransferMsg, TransferRequest,
};
use smart_explorer::types::{Conflict, CopyMode};
use smart_explorer::vfs::{Backend, BackendHandle};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set by the task suite"))
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

fn unique(name: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("{name}-{nanos}")
}

fn sftp() -> (BackendHandle, String) {
    let spec = env("SE_TASK_SFTP");
    let parts: Vec<&str> = spec.splitn(5, ':').collect();
    assert_eq!(
        parts.len(),
        5,
        "SE_TASK_SFTP is host:port:user:password:root"
    );
    let config = smart_explorer::sftp::SftpConfig {
        host: parts[0].to_string(),
        port: parts[1].parse().expect("SFTP port"),
        user: parts[2].to_string(),
        auth: smart_explorer::sftp::SftpAuth::Password(parts[3].to_string()),
        root: parts[4].to_string(),
    };
    let backend = smart_explorer::sftp::SftpBackend::connect(config).expect("SFTP connects");
    (Arc::new(backend), parts[4].to_string())
}

fn ftp() -> (BackendHandle, String) {
    let url = env("SE_TASK_FTP_URL");
    let backend = smart_explorer::ftp::backend_from_url(&url).expect("FTP connects");
    let root = url
        .splitn(4, '/')
        .nth(3)
        .map(|root| format!("/{root}"))
        .unwrap_or_else(|| "/".to_string());
    (Arc::new(backend), root)
}

fn local_tree(name: &str, folders: usize, files: usize, bytes: usize) -> PathBuf {
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

fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn job(
    source: Endpoint,
    target: Endpoint,
    paths: Vec<String>,
    target_dir: String,
    label: &str,
) -> TransferJob {
    TransferJob {
        source,
        target,
        target_dir,
        items: JobItems::Roots { paths, base: None },
        layout: Layout::Tree,
        filter: None,
        conflict: Conflict::Rename,
        mode: CopyMode::Copy,
        source_label: label.to_string(),
        target_label: label.to_string(),
        resume: None,
    }
}

struct Measured {
    first_file: Duration,
    total: Duration,
    files: u64,
    bytes: u64,
}

fn run(job: TransferJob, limit: Duration) -> Measured {
    let started = Instant::now();
    let mut active =
        launch_transfer(TransferRequest::Job(Box::new(job))).expect("the transfer starts");
    let mut first_file = None;
    loop {
        let left = limit.saturating_sub(started.elapsed());
        match active.rx.recv_timeout(left) {
            Ok(TransferMsg::Progress(progress)) => {
                if first_file.is_none() && progress.files_done > 0 {
                    first_file = Some(started.elapsed());
                }
            }
            Ok(TransferMsg::Done {
                progress,
                errors,
                canceled,
                ..
            }) => {
                let total = started.elapsed();
                if let Some(worker) = active.worker.take() {
                    let _ = worker.join();
                }
                assert!(!canceled, "the transfer was canceled");
                assert!(
                    errors.is_empty(),
                    "the transfer reported errors: {errors:?}"
                );
                return Measured {
                    first_file: first_file.unwrap_or(total),
                    total,
                    files: progress.files_done,
                    bytes: progress.bytes_done,
                };
            }
            Err(_) => {
                active.request_cancel();
                panic!("the transfer did not finish within {limit:?}");
            }
        }
    }
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

fn remote_folder(backend: &dyn Backend, root: &str, name: &str) -> String {
    let folder = format!("{}/{}", root.trim_end_matches('/'), unique(name));
    backend
        .mkdir_all(&folder)
        .expect("create remote case folder");
    folder
}

fn upload_small_files(case: &str, backend: BackendHandle, root: &str) {
    let link = link();
    let source = local_tree(case, SMALL_FOLDERS, SMALL_FILES, SMALL_FILE_BYTES);
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
    let source = local_tree(case, 1, 1, LARGE_FILE_BYTES);
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
        let mut writer = backend
            .open_write_new(&remote_file)
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
