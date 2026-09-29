//! Correctness of the transfer engine against real servers: whole trees go
//! up, across and down and arrive byte for byte with their empty folders and
//! non-ASCII names. `native/test-transfer-engine-task.sh` starts the
//! containers (see `transfer_task/support.rs`) and runs these ignored tests
//! with `--ignored --test-threads=1` on an unshaped loopback.
#[path = "transfer_task/support.rs"]
mod support;

use smart_explorer::transfer::Endpoint;
use smart_explorer::vfs::BackendHandle;
use std::time::Duration;
use support::{
    ftp, job, local_folder, local_tree, remote_folder, remote_tree, run, sample_tree, sftp, slash,
};

const TOP: &str = "Mappe mit Ümlaut";
const LIMIT: Duration = Duration::from_secs(300);

/// Upload the sample tree, compare it on the server, download it again and
/// compare it locally.
fn roundtrip(case: &str, backend: BackendHandle, root: &str) {
    let source = sample_tree(case);
    let expected = local_tree(&source);
    let remote = remote_folder(&*backend, root, case);
    let up = run(
        job(
            Endpoint::Local,
            Endpoint::Remote(backend.clone()),
            vec![slash(&source.join(TOP))],
            remote.clone(),
            case,
        ),
        LIMIT,
    );
    assert_eq!(up.files, 7, "{case}: every file uploaded");
    assert_eq!(
        remote_tree(&*backend, &remote),
        expected,
        "{case}: server tree"
    );
    let back = local_folder(case);
    let down = run(
        job(
            Endpoint::Remote(backend),
            Endpoint::Local,
            vec![format!("{remote}/{TOP}")],
            slash(&back),
            case,
        ),
        LIMIT,
    );
    assert_eq!(down.files, 7, "{case}: every file downloaded");
    assert_eq!(local_tree(&back), expected, "{case}: downloaded tree");
    let _ = std::fs::remove_dir_all(&source);
    let _ = std::fs::remove_dir_all(&back);
}

/// A second paste into the same folder keeps both: the folder gets a
/// numbered name and nothing of the first copy changes.
fn second_paste_keeps_both(case: &str, backend: BackendHandle, root: &str) {
    let source = sample_tree(case);
    let expected = local_tree(&source);
    let remote = remote_folder(&*backend, root, case);
    for _ in 0..2 {
        run(
            job(
                Endpoint::Local,
                Endpoint::Remote(backend.clone()),
                vec![slash(&source.join(TOP))],
                remote.clone(),
                case,
            ),
            LIMIT,
        );
    }
    let tree = remote_tree(&*backend, &remote);
    let numbered = format!("{TOP} (2)");
    assert!(tree.contains_key(TOP), "{case}: first copy stays");
    assert!(tree.contains_key(&numbered), "{case}: second copy numbered");
    for (rel, content) in &expected {
        let again = rel.replacen(TOP, &numbered, 1);
        assert_eq!(tree.get(rel), Some(content), "{case}: first copy unchanged");
        assert_eq!(
            tree.get(&again),
            Some(content),
            "{case}: second copy complete"
        );
    }
    let _ = std::fs::remove_dir_all(&source);
}

/// From one server to another (streamed through this process) and within
/// one server (server-side where the server offers it).
fn across(case: &str, from: BackendHandle, from_root: &str, to: BackendHandle, to_root: &str) {
    let source = sample_tree(case);
    let expected = local_tree(&source);
    let staged = remote_folder(&*from, from_root, case);
    run(
        job(
            Endpoint::Local,
            Endpoint::Remote(from.clone()),
            vec![slash(&source.join(TOP))],
            staged.clone(),
            case,
        ),
        LIMIT,
    );
    let target = remote_folder(&*to, to_root, case);
    let copied = run(
        job(
            Endpoint::Remote(from),
            Endpoint::Remote(to.clone()),
            vec![format!("{staged}/{TOP}")],
            target.clone(),
            case,
        ),
        LIMIT,
    );
    assert_eq!(copied.files, 7, "{case}: every file copied");
    assert_eq!(remote_tree(&*to, &target), expected, "{case}: copied tree");
    let _ = std::fs::remove_dir_all(&source);
}

#[test]
#[ignore = "needs the task suite's SFTP container"]
fn transfer_engine_task_container_sftp_tree_roundtrip() {
    let (backend, root) = sftp();
    roundtrip("sftp-roundtrip", backend, &root);
}

#[test]
#[ignore = "needs the task suite's FTP container"]
fn transfer_engine_task_container_ftp_tree_roundtrip() {
    let (backend, root) = ftp();
    roundtrip("ftp-roundtrip", backend, &root);
}

#[test]
#[ignore = "needs the task suite's SFTP container"]
fn transfer_engine_task_container_sftp_second_paste_keeps_both() {
    let (backend, root) = sftp();
    second_paste_keeps_both("sftp-twice", backend, &root);
}

#[test]
#[ignore = "needs the task suite's FTP container"]
fn transfer_engine_task_container_ftp_second_paste_keeps_both() {
    let (backend, root) = ftp();
    second_paste_keeps_both("ftp-twice", backend, &root);
}

#[test]
#[ignore = "needs the task suite's SFTP and FTP containers"]
fn transfer_engine_task_container_sftp_to_ftp_streams_the_tree() {
    let (from, from_root) = sftp();
    let (to, to_root) = ftp();
    across("sftp-to-ftp", from, &from_root, to, &to_root);
}

#[test]
#[ignore = "needs the task suite's SFTP container"]
fn transfer_engine_task_container_sftp_copy_within_one_server() {
    let (from, root) = sftp();
    // A second connection to the same account: one namespace, so the
    // engine copies on the server where it can.
    let (to, _) = sftp();
    across("sftp-within", from, &root, to, &root);
}
