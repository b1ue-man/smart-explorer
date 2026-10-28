//! Transfers with the crash log on against real servers (G3-01, G3-02,
//! G3-03): many small files go in packets both ways, every file arrives
//! byte for byte, and a second paste of the same files keeps both copies
//! under numbered names.
use crate::support::{
    ftp, job, local_folder, local_tree, remote_folder, remote_tree, run, sftp, slash, smb, webdav,
    with_crash_log,
};
use smart_explorer::transfer::Endpoint;
use smart_explorer::vfs::BackendHandle;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const LIMIT: Duration = Duration::from_secs(600);
const TOP: &str = "Pakete";
/// Small files per folder: several packets, members of both directions.
const SMALL_FILES: usize = 40;

fn content(index: usize, size: usize) -> Vec<u8> {
    (0..size)
        .map(|offset| (offset.wrapping_mul(13).wrapping_add(index * 7)) as u8)
        .collect()
}

fn write_file(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("create folders");
    std::fs::write(path, bytes).expect("write sample file");
}

/// Small files in the top folder and a subfolder (packet members), one
/// file of a few MiB (the single path with its stage and publication) and
/// an empty file.
fn packet_tree(name: &str) -> PathBuf {
    let root = local_folder(name);
    let top = root.join(TOP);
    for index in 0..SMALL_FILES {
        let small = top.join(format!("klein {index}.txt"));
        write_file(&small, &content(index, 4096));
        write_file(
            &top.join("unter").join(format!("datei-{index}.bin")),
            &content(index + 100, 1000 + index),
        );
    }
    write_file(&top.join("groß.bin"), &content(999, 5 * 1024 * 1024 + 3));
    write_file(&top.join("leer.txt"), &[]);
    root
}

/// Up and down with the crash log on: the tree arrives exactly.
fn roundtrip(case: &str, backend: BackendHandle, root: &str) {
    let source = packet_tree(case);
    let expected = local_tree(&source);
    let files = expected.values().filter(|entry| entry.is_some()).count() as u64;
    let remote = remote_folder(&*backend, root, case);
    let up = run(
        with_crash_log(job(
            Endpoint::Local,
            Endpoint::Remote(backend.clone()),
            vec![slash(&source.join(TOP))],
            remote.clone(),
            case,
        )),
        LIMIT,
    );
    assert_eq!(up.files, files, "{case}: every file uploaded");
    assert_eq!(
        remote_tree(&*backend, &remote),
        expected,
        "{case}: server tree"
    );
    let back = local_folder(case);
    let down = run(
        with_crash_log(job(
            Endpoint::Remote(backend),
            Endpoint::Local,
            vec![format!("{remote}/{TOP}")],
            slash(&back),
            case,
        )),
        LIMIT,
    );
    assert_eq!(down.files, files, "{case}: every file downloaded");
    assert_eq!(local_tree(&back), expected, "{case}: downloaded tree");
    let _ = std::fs::remove_dir_all(&source);
    let _ = std::fs::remove_dir_all(&back);
}

/// The same loose files pasted twice into one folder: the first copies stay,
/// the second ones get numbered names (packet members and the single path).
fn paste_twice(case: &str, backend: BackendHandle, root: &str) {
    let source = local_folder(case);
    let names = ["a.txt", "b.txt", "c.txt", "groß.bin"];
    let mut expected = BTreeMap::new();
    for (index, name) in names.iter().enumerate() {
        let size = if name.ends_with(".bin") {
            5 * 1024 * 1024 + 1
        } else {
            100 + index
        };
        let bytes = content(index, size);
        write_file(&source.join(name), &bytes);
        expected.insert(name.to_string(), Some(bytes));
    }
    let remote = remote_folder(&*backend, root, case);
    for _ in 0..2 {
        run(
            with_crash_log(job(
                Endpoint::Local,
                Endpoint::Remote(backend.clone()),
                names.iter().map(|name| slash(&source.join(name))).collect(),
                remote.clone(),
                case,
            )),
            LIMIT,
        );
    }
    let tree = remote_tree(&*backend, &remote);
    for (name, bytes) in &expected {
        let (stem, extension) = name.rsplit_once('.').expect("extension");
        let again = format!("{stem} (2).{extension}");
        assert_eq!(tree.get(name), Some(bytes), "{case}: first copy of {name}");
        assert_eq!(tree.get(&again), Some(bytes), "{case}: second copy {again}");
    }
    assert_eq!(
        tree.len(),
        2 * names.len(),
        "{case}: nothing else: {tree:?}"
    );
    let _ = std::fs::remove_dir_all(&source);
}

#[test]
#[ignore = "needs the task suite's SFTP container"]
fn transfer_task_crash_log_sftp_packets_roundtrip() {
    let (backend, root) = sftp();
    roundtrip("sftp-crash-log", backend, &root);
}

#[test]
#[ignore = "needs the task suite's SFTP container"]
fn transfer_task_crash_log_sftp_paste_twice_numbers_files() {
    let (backend, root) = sftp();
    paste_twice("sftp-crash-log-twice", backend, &root);
}

#[test]
#[ignore = "needs the task suite's Samba container"]
fn transfer_task_crash_log_smb_packets_roundtrip() {
    let Some((backend, root)) = smb() else {
        return;
    };
    roundtrip("smb-crash-log", backend, &root);
}

#[test]
#[ignore = "needs the task suite's FTP container"]
fn transfer_task_crash_log_ftp_roundtrip() {
    let (backend, root) = ftp();
    roundtrip("ftp-crash-log", backend, &root);
}

#[test]
#[ignore = "needs the task suite's FTP container"]
fn transfer_task_crash_log_ftp_paste_twice_numbers_files() {
    let (backend, root) = ftp();
    paste_twice("ftp-crash-log-twice", backend, &root);
}

#[test]
#[ignore = "needs the task suite's WebDAV server"]
fn transfer_task_crash_log_webdav_roundtrip() {
    let Some((backend, root)) = webdav() else {
        return;
    };
    roundtrip("webdav-crash-log", backend, &root);
}

#[test]
#[ignore = "needs the task suite's WebDAV server"]
fn transfer_task_crash_log_webdav_paste_twice_numbers_files() {
    let Some((backend, root)) = webdav() else {
        return;
    };
    paste_twice("webdav-crash-log-twice", backend, &root);
}
