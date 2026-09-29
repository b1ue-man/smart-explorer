//! Shared helpers of the transfer-engine task tests against real servers
//! (`transfer_throughput.rs`, `transfer_containers.rs`). The task suite
//! (`native/test-transfer-engine-task.sh`) starts the containers and names
//! them in the environment:
//!
//! - `SE_TASK_SFTP`: `host:port:user:password:root` of an SFTP server.
//! - `SE_TASK_FTP_URL`: `ftp://user:password@host:port/root`.
#![allow(dead_code)]

use smart_explorer::transfer::{
    launch_transfer, Endpoint, JobItems, Layout, TransferJob, TransferMsg, TransferRequest,
};
use smart_explorer::types::{Conflict, CopyMode};
use smart_explorer::vfs::{Backend, BackendHandle};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set by the task suite"))
}

pub fn unique(name: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("{name}-{nanos}")
}

/// A connected SFTP backend and its writable root.
pub fn sftp() -> (BackendHandle, String) {
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

/// A connected FTP backend and its writable root.
pub fn ftp() -> (BackendHandle, String) {
    let url = env("SE_TASK_FTP_URL");
    let backend = smart_explorer::ftp::backend_from_url(&url).expect("FTP connects");
    let root = url
        .splitn(4, '/')
        .nth(3)
        .map(|root| format!("/{root}"))
        .unwrap_or_else(|| "/".to_string());
    (Arc::new(backend), root)
}

/// Forward-slash form of a local path, as jobs take it.
pub fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// A new, empty local folder below the temp directory.
pub fn local_folder(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(unique(name));
    std::fs::create_dir_all(&folder).expect("create local folder");
    folder
}

/// A new folder below `root` on `backend`.
pub fn remote_folder(backend: &dyn Backend, root: &str, name: &str) -> String {
    let folder = format!("{}/{}", root.trim_end_matches('/'), unique(name));
    backend
        .mkdir_all(&folder)
        .expect("create remote case folder");
    folder
}

pub fn job(
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

/// What one finished job did.
pub struct Measured {
    pub first_file: Duration,
    pub total: Duration,
    pub files: u64,
    pub bytes: u64,
}

/// Runs `job` through the transfer lane's launcher (as the app does) and
/// waits at most `limit`; any issue, cancel or timeout fails the test.
pub fn run(job: TransferJob, limit: Duration) -> Measured {
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

/// Every file below a local folder: relative path → content; folders as
/// `None` so empty folders count too.
pub fn local_tree(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    let mut tree = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(folder) = pending.pop() {
        for entry in std::fs::read_dir(&folder).expect("list local folder") {
            let entry = entry.expect("local entry");
            let path = entry.path();
            let rel = slash(path.strip_prefix(root).expect("below root"));
            if entry.file_type().expect("entry type").is_dir() {
                tree.insert(rel, None);
                pending.push(path);
            } else {
                tree.insert(rel, Some(std::fs::read(&path).expect("read local file")));
            }
        }
    }
    tree
}

/// The same view of a folder on `backend`.
pub fn remote_tree(backend: &dyn Backend, root: &str) -> BTreeMap<String, Option<Vec<u8>>> {
    let mut tree = BTreeMap::new();
    let mut pending = vec![String::new()];
    while let Some(rel) = pending.pop() {
        let folder = if rel.is_empty() {
            root.to_string()
        } else {
            format!("{}/{rel}", root.trim_end_matches('/'))
        };
        for meta in backend.list_dir(&folder).expect("list remote folder") {
            let child = if rel.is_empty() {
                meta.name.clone()
            } else {
                format!("{rel}/{}", meta.name)
            };
            let path = format!("{}/{}", folder.trim_end_matches('/'), meta.name);
            if meta.is_dir {
                tree.insert(child.clone(), None);
                pending.push(child);
            } else {
                let mut bytes = Vec::new();
                backend
                    .open_read(&path)
                    .expect("open remote file")
                    .read_to_end(&mut bytes)
                    .expect("read remote file");
                tree.insert(child, Some(bytes));
            }
        }
    }
    tree
}

/// A tree with the shapes transfers must keep: nested folders, an empty
/// folder, an empty file, sizes around the usual block boundaries and names
/// with spaces and non-ASCII letters.
pub fn sample_tree(name: &str) -> PathBuf {
    let root = local_folder(name);
    let top = root.join("Mappe mit Ümlaut");
    let files: [(&str, usize); 7] = [
        ("leer.txt", 0),
        ("klein.txt", 5),
        ("block.bin", 32 * 1024),
        ("block+1.bin", 32 * 1024 + 1),
        ("unter/tief/datei.bin", 300 * 1024),
        ("unter/groß.bin", 3 * 1024 * 1024 + 7),
        ("unter/tief/noch tiefer/ende.txt", 17),
    ];
    for (index, (rel, size)) in files.iter().enumerate() {
        let path = top.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create folders");
        let content: Vec<u8> = (0..*size)
            .map(|offset| (offset.wrapping_mul(31).wrapping_add(index)) as u8)
            .collect();
        let mut file = std::fs::File::create(&path).expect("create sample file");
        file.write_all(&content).expect("write sample file");
    }
    std::fs::create_dir_all(top.join("leerer Ordner")).expect("create empty folder");
    root
}
