//! `speed_task_remote_*`: the G5 cases of the remote backends that run on a
//! CI runner (analysis G3, section 12). Each case uploads a local tree,
//! downloads it again and copies it within the same server, once without
//! and once with the crash log, and prints one line per measurement:
//!
//! `BENCH <backend>_<case>_<up|down|same_server>_<plain|crash_log> files_per_s=<n> mib_per_s=<n>`
//!
//! The rates count from submitting the job to its end (first to last byte,
//! setup included). No bound is checked here: the numbers are compared with
//! the reference tools on the same runner (`rsync`, `rclone`, `lftp`,
//! `smbclient`, `curl`). Only the file count is asserted, so a broken
//! transfer cannot pass as fast.
use crate::support::{
    ftp, job, local_folder, nextcloud, remote_folder, run, sftp, sftp_stored, slash, smb, webdav,
    with_crash_log, Measured,
};
use smart_explorer::transfer::Endpoint;
use smart_explorer::vfs::BackendHandle;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A benchmark job may take this long before the case fails (2 GiB over a
/// slow shaped link).
const LIMIT: Duration = Duration::from_secs(3 * 60 * 60);
pub(crate) const TOP: &str = "bench";
const MIB: f64 = 1024.0 * 1024.0;

/// One G5 case: `files` files of `bytes` bytes in `folders` folders.
pub(crate) struct Case {
    name: &'static str,
    folders: usize,
    files: usize,
    bytes: usize,
}

const SMALL_ONE_FOLDER: Case = Case {
    name: "small_one_folder",
    folders: 1,
    files: 5_000,
    bytes: 4 * 1024,
};
const SMALL_50_FOLDERS: Case = Case {
    name: "small_50_folders",
    folders: 50,
    files: 5_000,
    bytes: 4 * 1024,
};
const MEDIUM: Case = Case {
    name: "medium",
    folders: 1,
    files: 1_000,
    bytes: 1024 * 1024,
};
/// G3-07: single files above the packet size (2 MiB) that still fit one
/// SMB WRITE of a large-MTU server (8 MiB): each goes as one compound.
const SINGLE_COMPOUND: Case = Case {
    name: "single_compound",
    folders: 1,
    files: 200,
    bytes: 3 * 1024 * 1024,
};
/// G5 names 10 GiB; a CI runner's disk and time allow 2 GiB.
pub(crate) const LARGE: Case = Case {
    name: "large",
    folders: 1,
    files: 1,
    bytes: 2 * 1024 * 1024 * 1024,
};

/// Incompressible bytes (SSH may compress), different per file.
fn fill(file: &mut std::fs::File, mut bytes: usize, seed: u64) {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut block = vec![0u8; 1024 * 1024];
    while bytes > 0 {
        for chunk in block.chunks_mut(8) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let word = state.to_le_bytes();
            chunk.copy_from_slice(&word[..chunk.len()]);
        }
        let take = bytes.min(block.len());
        file.write_all(&block[..take]).expect("write bench file");
        bytes -= take;
    }
}

pub(crate) fn local_case(case: &Case, backend: &str) -> PathBuf {
    let root = local_folder(&format!("bench-{backend}-{}", case.name));
    for folder in 0..case.folders {
        std::fs::create_dir_all(root.join(TOP).join(format!("d{folder}")))
            .expect("create bench folder");
    }
    for index in 0..case.files {
        let folder = format!("d{}", index % case.folders);
        let path = root.join(TOP).join(folder).join(format!("f{index}.bin"));
        let mut file = std::fs::File::create(&path).expect("create bench file");
        fill(&mut file, case.bytes, index as u64 + 1);
    }
    root
}

pub(crate) fn report(label: &str, measured: &Measured, files: usize) {
    let seconds = measured.total.as_secs_f64().max(f64::EPSILON);
    println!(
        "BENCH {label} files_per_s={:.1} mib_per_s={:.2}",
        measured.files as f64 / seconds,
        measured.bytes as f64 / MIB / seconds
    );
    assert_eq!(measured.files, files as u64, "{label}: every file moved");
}

pub(crate) fn bench_job(crash_log: bool, job: smart_explorer::transfer::TransferJob) -> Measured {
    let job = if crash_log { with_crash_log(job) } else { job };
    run(job, LIMIT)
}

/// Up, down and within the server, without and with the crash log.
fn bench(backend_name: &str, backend: BackendHandle, root: &str, case: &Case) {
    let source = local_case(case, backend_name);
    for crash_log in [false, true] {
        let mode = if crash_log { "crash_log" } else { "plain" };
        let label = |direction: &str| format!("{backend_name}_{}_{direction}_{mode}", case.name);
        let remote = remote_folder(&*backend, root, &format!("bench-{}", case.name));
        let up = bench_job(
            crash_log,
            job(
                Endpoint::Local,
                Endpoint::Remote(backend.clone()),
                vec![slash(&source.join(TOP))],
                remote.clone(),
                "bench",
            ),
        );
        report(&label("up"), &up, case.files);
        let back = local_folder(&format!("bench-back-{backend_name}"));
        let down = bench_job(
            crash_log,
            job(
                Endpoint::Remote(backend.clone()),
                Endpoint::Local,
                vec![format!("{remote}/{TOP}")],
                slash(&back),
                "bench",
            ),
        );
        report(&label("down"), &down, case.files);
        let _ = std::fs::remove_dir_all(&back);
        let copy = remote_folder(&*backend, root, &format!("bench-copy-{}", case.name));
        let within = bench_job(
            crash_log,
            job(
                Endpoint::Remote(backend.clone()),
                Endpoint::Remote(backend.clone()),
                vec![format!("{remote}/{TOP}")],
                copy,
                "bench",
            ),
        );
        report(&label("same_server"), &within, case.files);
    }
    remove_local(&source);
}

fn remove_local(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

macro_rules! remote_benchmarks {
    ($backend:literal, $connect:expr, $($test:ident => $case:expr),+ $(,)?) => {
        $(
            #[test]
            #[ignore = "benchmark: needs the task suite's servers"]
            fn $test() {
                let Some((backend, root)) = $connect else {
                    return;
                };
                bench($backend, backend, &root, &$case);
            }
        )+
    };
}

remote_benchmarks!(
    "sftp",
    Some(sftp()),
    speed_task_remote_sftp_small_one_folder => SMALL_ONE_FOLDER,
    speed_task_remote_sftp_small_50_folders => SMALL_50_FOLDERS,
    speed_task_remote_sftp_medium => MEDIUM,
    speed_task_remote_sftp_large => LARGE,
);

remote_benchmarks!(
    "smb",
    smb(),
    speed_task_remote_smb_small_one_folder => SMALL_ONE_FOLDER,
    speed_task_remote_smb_small_50_folders => SMALL_50_FOLDERS,
    speed_task_remote_smb_medium => MEDIUM,
    speed_task_remote_smb_large => LARGE,
    speed_task_remote_smb_single_compound => SINGLE_COMPOUND,
);

remote_benchmarks!(
    "ftp",
    Some(ftp()),
    speed_task_remote_ftp_small_one_folder => SMALL_ONE_FOLDER,
    speed_task_remote_ftp_small_50_folders => SMALL_50_FOLDERS,
    speed_task_remote_ftp_medium => MEDIUM,
    speed_task_remote_ftp_large => LARGE,
);

remote_benchmarks!(
    "webdav",
    webdav(),
    speed_task_remote_webdav_small_one_folder => SMALL_ONE_FOLDER,
    speed_task_remote_webdav_small_50_folders => SMALL_50_FOLDERS,
    speed_task_remote_webdav_medium => MEDIUM,
    speed_task_remote_webdav_large => LARGE,
);

// G3-12: the same cases on a Nextcloud, whose large files go up as chunks.
remote_benchmarks!(
    "nextcloud",
    nextcloud(),
    speed_task_remote_nextcloud_small_one_folder => SMALL_ONE_FOLDER,
    speed_task_remote_nextcloud_medium => MEDIUM,
    speed_task_remote_nextcloud_large => LARGE,
);

/// G3-10: one large file up and down with a login that needed no input, so
/// extra SSH connections (each with its own TCP stream) may carry ranges.
/// Right after each job the note must name more than one connection and no
/// block: the extra connections logged in with the main connection's host
/// key (a refused key would show "gesperrt").
#[test]
#[ignore = "benchmark: needs the task suite's servers"]
fn speed_task_remote_sftp_large_extra_connections() {
    let (backend, root) = sftp_stored();
    let source = local_case(&LARGE, "sftp_extra");
    let remote = remote_folder(&*backend, &root, "bench-extra-large");
    let up = bench_job(
        false,
        job(
            Endpoint::Local,
            Endpoint::Remote(backend.clone()),
            vec![slash(&source.join(TOP))],
            remote.clone(),
            "bench",
        ),
    );
    report("sftp_extra_connections_large_up_plain", &up, LARGE.files);
    assert_extra_connections(&*backend);
    let back = local_folder("bench-back-sftp-extra");
    let down = bench_job(
        false,
        job(
            Endpoint::Remote(backend.clone()),
            Endpoint::Local,
            vec![format!("{remote}/{TOP}")],
            slash(&back),
            "bench",
        ),
    );
    report(
        "sftp_extra_connections_large_down_plain",
        &down,
        LARGE.files,
    );
    assert_extra_connections(&*backend);
    let _ = std::fs::remove_dir_all(&back);
    remove_local(&source);
}

fn assert_extra_connections(backend: &dyn smart_explorer::vfs::Backend) {
    let hint = backend.transfer_hint().unwrap_or_default();
    println!("NOTE sftp_extra_connections {hint:?}");
    assert!(
        !hint.contains("gesperrt"),
        "an extra connection failed: {hint}"
    );
    assert!(
        hint.contains("Verbindungen zu"),
        "the transfer used extra connections: {hint:?}"
    );
}

/// G3-11: a large FTP download in a fresh job (no free logged-in connection
/// yet) runs in ranges over connections logged in for them; the downloaded
/// file must equal the source byte for byte.
#[test]
#[ignore = "benchmark: needs the task suite's servers"]
fn speed_task_remote_ftp_large_download_ranges_fresh_job() {
    let (seed, root) = ftp();
    let source = local_case(&LARGE, "ftp_ranges");
    let remote = remote_folder(&*seed, &root, "bench-ftp-ranges");
    let up = bench_job(
        false,
        job(
            Endpoint::Local,
            Endpoint::Remote(seed.clone()),
            vec![slash(&source.join(TOP))],
            remote.clone(),
            "bench",
        ),
    );
    report("ftp_ranges_large_up_plain", &up, LARGE.files);
    drop(seed);
    // A new backend: its pool holds no logged-in transfer connection.
    let (fresh, _) = ftp();
    let back = local_folder("bench-back-ftp-ranges");
    let down = bench_job(
        false,
        job(
            Endpoint::Remote(fresh),
            Endpoint::Local,
            vec![format!("{remote}/{TOP}")],
            slash(&back),
            "bench",
        ),
    );
    report("ftp_ranges_large_down_fresh_plain", &down, LARGE.files);
    let file = Path::new(TOP).join("d0").join("f0.bin");
    assert!(
        same_bytes(&source.join(&file), &back.join(&file)),
        "the ranges assembled the file byte for byte"
    );
    let _ = std::fs::remove_dir_all(&back);
    remove_local(&source);
}

/// Two files compared in 1 MiB blocks (a 2 GiB file is never held whole).
pub(crate) fn same_bytes(left: &Path, right: &Path) -> bool {
    use std::io::Read;
    let (Ok(mut left), Ok(mut right)) = (std::fs::File::open(left), std::fs::File::open(right))
    else {
        return false;
    };
    let (mut a, mut b) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
    loop {
        let read = match left.read(&mut a) {
            Ok(read) => read,
            Err(_) => return false,
        };
        if right.read_exact(&mut b[..read]).is_err() || a[..read] != b[..read] {
            return false;
        }
        if read == 0 {
            return right.read(&mut b[..1]).is_ok_and(|more| more == 0);
        }
    }
}
