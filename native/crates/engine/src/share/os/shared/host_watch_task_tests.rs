//! R-FS-2 and E11 (plan 2026-11-07), watch sessions: a watch under an export
//! reports nothing from the app's private places or from an unlocked vault
//! drive, and a watch whose root is such a place is a permission refusal.
use super::*;
use crate::share::{
    export_config::ExportAccess,
    fs::{ShareExportConfig, SharedRoot},
    fs_host_policy_task_tests::{text, vault_fixture, Published},
};
use std::{path::Path, time::Instant};

const WAIT: Duration = Duration::from_secs(10);

fn access(folder: &Path) -> FsAccess {
    FsAccess::dynamic(ShareExportConfig {
        roots: vec![SharedRoot::new("Docs", text(folder)).with_access(ExportAccess::ReadWrite)],
        ..Default::default()
    })
}

fn start(path: &str, access: &FsAccess) -> io::Result<Session> {
    setup(
        FsWatch { path: path.into() },
        access.clone(),
        Progress::default(),
        crate::share::analysis_resources::Reservation::metadata(4096)?,
    )
}

/// Waits until every local watch of the session reported `Ready`.
fn armed(session: &Session) {
    let deadline = Instant::now() + WAIT;
    let mut ready = 0;
    while ready < session.local.len() {
        let left = deadline.saturating_duration_since(Instant::now());
        let message = session
            .rx
            .recv_timeout(left)
            .expect("the watch is armed in time");
        if matches!(message.event, watch::WatchEvent::Ready(_)) {
            ready += 1;
        }
    }
}

/// The paths changed until `stop` accepts the list or `limit` has passed.
fn changes_until(
    session: &Session,
    limit: Duration,
    stop: impl Fn(&[String]) -> bool,
) -> Vec<String> {
    let deadline = Instant::now() + limit;
    let mut changed = Vec::new();
    while Instant::now() < deadline && !stop(&changed) {
        if let Ok(message) = session.rx.recv_timeout(Duration::from_millis(50)) {
            if let watch::WatchEvent::Change(change) = message.event {
                changed.push(change.rel);
            }
        }
    }
    changed
}

/// M16-d: under a watch of the export, a change in the unlocked vault drive
/// is not reported, a change of an ordinary file is.
#[test]
fn vault_task_watch_reports_nothing_from_an_unlocked_vault_drive() {
    let (_directory, folder, drive) = vault_fixture();
    let _published = Published::new(&drive);
    let session = start("/Docs", &access(&folder)).expect("the export is watched");
    armed(&session);
    std::fs::write(drive.join("neu.txt"), b"klartext").unwrap();
    std::fs::write(drive.join("geheim.txt"), b"anders").unwrap();
    std::fs::write(folder.join("frei.txt"), b"anders").unwrap();
    let mut changed = changes_until(&session, WAIT, |seen| {
        seen.iter().any(|rel| rel == "frei.txt")
    });
    assert!(changed.iter().any(|rel| rel == "frei.txt"), "{changed:?}");
    // The drive's changes were made before the file's: they would be here.
    changed.extend(changes_until(&session, Duration::from_millis(300), |_| {
        false
    }));
    assert!(
        !changed
            .iter()
            .any(|rel| rel == "T" || rel.starts_with("T/")),
        "{changed:?}"
    );
}

/// M16-d: a watch whose root is the drive or lies in it keeps its refusal as
/// a permission error (it was a plain failure); once the drive ended the
/// folder can be watched.
#[test]
fn vault_task_watch_on_an_unlocked_vault_drive_is_a_permission_refusal() {
    let (_directory, folder, drive) = vault_fixture();
    let access = access(&folder);
    let published = Published::new(&drive);
    for path in ["/Docs/T", "/Docs/T/sub"] {
        let error = start(path, &access).err().expect("the watch is refused");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{path}");
    }
    assert_watch_starts("/Docs", &access);
    drop(published);
    assert_watch_starts("/Docs/T", &access);
}

/// Asserts that a watch of `path` starts on every host platform, Windows
/// included: there a local share is watched through the held folder handle.
fn assert_watch_starts(path: &str, access: &FsAccess) {
    if let Err(error) = start(path, access) {
        panic!("the watch of {path} does not start: {error}");
    }
}

/// R-FS-2 (T-R-FS-2): the filter of a watch drops the app's data folder and
/// everything in it, and nothing next to it. The folder lies in the data
/// home, which no test can share without a process-wide setting, so this
/// asks the filter's decision for the real private places; the same filter
/// carries the vault test above.
#[test]
fn review_task_watch_filter_drops_the_app_data_folder_and_only_that() {
    let data = crate::support_dirs::app_data_dir();
    let data = std::fs::canonicalize(&data).unwrap_or(data);
    let home = data.parent().expect("the data folder has a parent");
    let name = data.file_name().unwrap().to_string_lossy().into_owned();
    let excluded = private_forms(&storage_roots::excluded());
    for inside in [
        name.clone(),
        format!("{name}/identity.json"),
        format!("{name}/sync/jobs/a.json"),
    ] {
        assert!(watch_excluded(home, &inside, &excluded), "{inside}");
    }
    for outside in [
        "Documents/a.txt".to_string(),
        format!("{name}-other/a.txt"),
        "a.txt".to_string(),
    ] {
        assert!(!watch_excluded(home, &outside, &excluded), "{outside}");
    }
}
