//! Linux drives in the daemon (stage `linux-root: drive-fuse`, fixture
//! `home` for a fresh HOME): SMB stays refused as a drive source (D3,
//! preservation of S2), and the Linux FUSE part of
//! `drive_task_auto_remount_after_daemon_restart_fuse` (DRV-M3): a worker of the
//! built `se` (`SMART_EXPLORER_DRIVE_SE_BIN`) mounts a saved SFTP drive with
//! `--auto-remount`; after that worker stopped, the start of the login
//! entry (`se --sync-daemon --drive-restore`) mounts it again at the same
//! folder. Both tests are `#[ignore]` (`SMART_EXPLORER_FUSE_TASK=1`).
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::fuse_task_backends::{sftp, variable};
use super::fuse_task_harness::{
    eventually, mounted, program, remove_remote_tree, require_task_stage,
};
use crate::creds::{AuthKind, Protocol, SavedConnection};
use crate::mount::{
    BackendRoot, DriveSelection, MountConfig, MountId, MountMode, MountSource, MountStatus,
};

const SE_BIN: &str = "SMART_EXPLORER_DRIVE_SE_BIN";
const LABEL: &str = "fuse-e2e";
const CONTENT: &[u8] = b"mounted again after the restart";
const START_LIMIT: Duration = Duration::from_secs(120);

/// Saved connections, secrets and drives of these tests live in the fresh
/// HOME of the fixture `home`, never in the runner user's own.
fn require_isolated_home() {
    for name in ["HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME"] {
        assert!(
            std::env::var_os(name).is_some(),
            "{name} fehlt: diese Prüfung braucht die Fixture home der Stufe linux-root: drive-fuse"
        );
    }
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse with the home fixture (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_smb_mount_still_refused() -> io::Result<()> {
    require_task_stage();
    require_isolated_home();
    let connection = SavedConnection {
        protocol: Protocol::Smb,
        host: "nas.example.test".into(),
        port: Protocol::Smb.default_port(),
        user: "user".into(),
        auth: AuthKind::Password,
        root: "/share".into(),
        label: "nas".into(),
        use_agent: false,
    };
    crate::creds::save_connection(&connection)?;
    let config = MountConfig::new(
        MountId::new_random()?,
        MountSource::SavedRemote {
            account: connection.account(),
            root: BackendRoot::parse("/share")?,
        },
        DriveSelection::Automatic,
        MountMode::ReadOnly,
        "nas",
    )?;
    let id = config.id.clone();
    // The start path of the daemon, which now continues on Linux.
    let host = super::super::ipc_host::ShareHost::new("0".repeat(32));
    let snapshot = host.mounts.start(config, &host).map_err(io::Error::other)?;
    assert_eq!(
        snapshot.status,
        MountStatus::Failed {
            detail: "SMB lässt sich nicht als Laufwerk einbinden".into()
        }
    );
    // Nothing was mounted, and nothing was registered for a later start.
    if let Some(folder) = snapshot.config.location_directory() {
        assert!(!mounted(folder));
    }
    assert!(!super::super::mount_registry::registered_for_completion()
        .iter()
        .any(|registered| registered.id == id));
    Ok(())
}

/// Stops the worker of this HOME and waits until no worker holds the
/// single-instance lock.
fn stop_workers() -> io::Result<()> {
    super::super::state::request_stop()?;
    match super::super::platform::acquire_daemon_instance_guard(START_LIMIT) {
        Some(guard) => {
            drop(guard);
            Ok(())
        }
        None => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "the background worker did not stop",
        )),
    }
}

/// `se drive list` (text), for the failure messages.
fn listing(se: &Path) -> String {
    program(Command::new(se).args(["drive", "list"]))
        .map(|(code, output)| format!("exit {code}: {output}"))
        .unwrap_or_else(|error| error.to_string())
}

fn end_child(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse with se, sftp and home (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_auto_remount_after_daemon_restart_fuse() -> io::Result<()> {
    require_task_stage();
    require_isolated_home();
    let se = std::env::var_os(SE_BIN)
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{SE_BIN} fehlt")))?;
    // The server folder with one file, and the saved connection `se` uses
    // (host key pinned by `sftp`, password in the sealed store).
    let (raw, _drive, server) = sftp(MountMode::ReadWrite)?;
    let mut writer = raw.open_write(&format!("{server}/restart.txt"))?;
    io::Write::write_all(&mut writer, CONTENT)?;
    io::Write::flush(&mut writer)?;
    drop(writer);
    let connection = SavedConnection {
        protocol: Protocol::Sftp,
        host: variable("SE_TASK_SFTP_HOST")?,
        port: variable("SE_TASK_SFTP_PORT")?
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "SE_TASK_SFTP_PORT"))?,
        user: variable("SE_TASK_SFTP_USER")?,
        auth: AuthKind::Password,
        root: server.clone(),
        label: LABEL.into(),
        use_agent: false,
    };
    let password = variable("SE_TASK_SFTP_PASSWORD")?;
    crate::creds::save_connection_with_secret(&connection, Some(&password))
        .map_err(io::Error::other)?;
    let base = tempfile::tempdir()?;
    let folder = base.path().join("drive");
    let outcome = remount_round_trip(&se, &folder);
    // Whatever happened: no worker, mount or server folder stays behind.
    let stopped = stop_workers();
    if mounted(&folder) {
        let _ = crate::mount::unmount_drive_folder(&folder, true);
    }
    let _ = remove_remote_tree(&raw, &server);
    outcome.and(stopped)
}

fn remount_round_trip(se: &Path, folder: &Path) -> io::Result<()> {
    // First worker, started by `se drive mount`.
    let (code, output) = program(
        Command::new(se)
            .args(["drive", "mount"])
            .arg(format!("@{LABEL}:/"))
            .arg("--mount-point")
            .arg(folder)
            .args(["--read-write", "--trust-remote-root", "--auto-remount"]),
    )?;
    assert_eq!(code, 0, "se drive mount: {output}");
    assert!(
        eventually(START_LIMIT, || mounted(folder)),
        "first mount: {}",
        listing(se)
    );
    assert_eq!(fs::read(folder.join("restart.txt"))?, CONTENT);
    // The worker stops; its host ends like at an update stop and unmounts,
    // the drive stays registered for the restore.
    stop_workers()?;
    assert!(
        !mounted(folder),
        "the stopped worker left the drive mounted"
    );
    // The start of the login entry.
    let mut restore = Command::new(se)
        .args(["--sync-daemon", "--drive-restore"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let remounted = eventually(START_LIMIT, || mounted(folder));
    let shown = listing(se);
    let content = fs::read(folder.join("restart.txt"));
    let stopped = stop_workers();
    end_child(&mut restore);
    assert!(remounted, "no mount after the restart: {shown}");
    assert!(
        shown.contains(&format!("mounted:{}", folder.display())),
        "{shown}"
    );
    assert_eq!(content?, CONTENT);
    stopped
}
