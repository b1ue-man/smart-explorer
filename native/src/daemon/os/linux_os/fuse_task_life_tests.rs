//! Signals of DRV-M4 (strang-laufwerke.md L 264), part 3: the life of a
//! Linux drive: update stop (D3, Linux variant of the DRV-M2 signal), eject
//! of a busy drive, `--force` through `fusectl`, the kernel request timeout,
//! a killed host, a stale mount, the locked folder, and Retry after a
//! restart. Every test mounts through `fusermount3` and is `#[ignore]`:
//! stage `linux-root: drive-fuse` (`SMART_EXPLORER_FUSE_TASK=1`, fixture
//! `fuse`; the kernel timeout also `fuse-timeout`).
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use super::super::mount_host_process::MountHostProcess;
use super::fuse_task_backends::{memory, rooted, variable, Gated, GatedBackend};
use super::fuse_task_harness::{
    clean_end, close_checked, eventually, last, mounted, names_in, require_task_stage,
    unmount_cleanly, FuseTaskHost, HostSetup, Places, MOUNT_LIMIT, SWITCH,
};
use crate::mount::optimization_fixture::OptimizationBackend;
use crate::mount::os::linux_os::{fuse_abort, mount_point};
use crate::mount::os::shared::host_session::HostExit;
use crate::mount::{MountMode, MountRecovery, MountStatus};

const RW: MountMode = MountMode::ReadWrite;
const CHILD_CACHE: &str = "SMART_EXPLORER_FUSE_TASK_CHILD_CACHE";
const CHILD_FOLDER: &str = "SMART_EXPLORER_FUSE_TASK_CHILD_FOLDER";
const CHILD_TEST: &str = "fuse_task_child_host_serves_until_killed";

fn mode_of(path: &Path) -> io::Result<u32> {
    Ok(fs::symlink_metadata(path)?.permissions().mode() & 0o7777)
}

/// `--force` aborts through `fusectl` (`/sys/fs/fuse/connections`, mounted
/// by systemd at boot); its absence is named instead of a hanging reader.
fn require_fusectl(folder: &Path) {
    let canonical = mount_point::canonical(folder).unwrap_or_else(|_| folder.to_path_buf());
    let connection = fuse_abort::connection_at(&canonical).expect("the drive has a connection");
    let abort = Path::new("/sys/fs/fuse/connections")
        .join(connection.to_string())
        .join("abort");
    assert!(
        abort.exists(),
        "{} fehlt: fusectl ist auf dem Runner nicht eingehängt",
        abort.display()
    );
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_host_session_update_stop_unmounts_and_exits() -> io::Result<()> {
    require_task_stage();
    let (fixture, drive) = memory(RW)?;
    fixture.put("/note.txt", b"before");
    let host = FuseTaskHost::start(HostSetup::new(drive, RW))?;
    let folder = host.folder.clone();
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(host.path("note.txt"))?;
    file.write_all(b"after the update stop")?;
    drop(file);
    let (result, reports) = host.finish(true)?;
    assert_eq!(result, Ok(HostExit::UpdateStop));
    assert_eq!(HostExit::UpdateStop.exit_code(), 75);
    assert_eq!(fixture.bytes("/note.txt"), b"after the update stop");
    assert!(
        !mounted(&folder),
        "the drive stayed mounted after the update stop"
    );
    assert_eq!(last(&reports), Some(&clean_end()));
    Ok(())
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_fuse_unmount_busy_then_force() -> io::Result<()> {
    require_task_stage();
    let (fixture, drive) = memory(RW)?;
    fixture.put("/open.txt", b"held open");
    let mut host = FuseTaskHost::start(HostSetup::new(drive, RW))?;
    let folder = host.folder.clone();
    let held = File::open(host.path("open.txt"))?;
    // Eject, as a file manager does it: a busy drive stays mounted.
    let busy = crate::mount::unmount_drive_folder(&folder, false).expect_err("busy");
    assert_eq!(busy.kind(), io::ErrorKind::ResourceBusy, "{busy}");
    assert!(mounted(&folder), "a busy drive stays mounted");
    assert_eq!(fs::read(host.path("open.txt"))?, b"held open");
    // --force: unmounted lazily, the connection aborted.
    require_fusectl(&folder);
    crate::mount::unmount_drive_folder(&folder, true)?;
    assert!(!mounted(&folder));
    drop(held);
    // The host sees an outside unmount and ends like at an unmount request.
    let (result, reports) = host.end()?;
    assert_eq!(result, Ok(HostExit::Stopped));
    assert_eq!(last(&reports), Some(&clean_end()));
    assert_eq!(mode_of(&folder)?, 0o500);
    Ok(())
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_fuse_force_abort_frees_blocked_reader() -> io::Result<()> {
    require_task_stage();
    let gated = GatedBackend::new();
    gated.inner.put("/big.bin", &vec![7u8; 1024 * 1024]);
    let mut host = FuseTaskHost::start(HostSetup::new(gated.drive(RW)?, RW))?;
    let folder = host.folder.clone();
    let gate = gated.gate(Gated::Read("/big.bin".into()));
    let path = host.path("big.bin");
    let (done, reader) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(fs::read(path));
    });
    assert!(
        gate.entered_within(Duration::from_secs(30)),
        "the read never reached the backend"
    );
    assert!(
        reader.recv_timeout(Duration::from_secs(1)).is_err(),
        "the reader must still wait for the backend"
    );
    // S2-A5: --force aborts the connection through fusectl; the program
    // blocked in the drive gets an error instead of hanging.
    require_fusectl(&folder);
    crate::mount::unmount_drive_folder(&folder, true)?;
    let freed = reader
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| io::Error::other("the blocked reader was not freed"))?;
    let error = freed.expect_err("an aborted read fails");
    // `read(2)` goes through the page cache: the kernel ends the aborted
    // request with ECONNABORTED, but reports a page that stayed unread as EIO
    // (observed on the runner's kernel 7.0); a later call sees ENOTCONN.
    assert!(
        matches!(
            error.raw_os_error(),
            Some(libc::ECONNABORTED) | Some(libc::ENOTCONN) | Some(libc::EIO)
        ),
        "{error}"
    );
    gate.release(false);
    let (result, reports) = host.end()?;
    assert_eq!(result, Ok(HostExit::Stopped));
    assert_eq!(last(&reports), Some(&clean_end()));
    assert!(!mounted(&folder));
    Ok(())
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse with fs.fuse.default_request_timeout (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_fuse_kernel_request_timeout_reported() -> io::Result<()> {
    require_task_stage();
    let seconds: u64 = variable("SE_TASK_FUSE_REQUEST_TIMEOUT_S")?
        .parse()
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "SE_TASK_FUSE_REQUEST_TIMEOUT_S",
            )
        })?;
    assert_eq!(
        crate::mount::os::linux_os::fuse_runtime::kernel_request_timeout(),
        Some(seconds),
        "the fixture fuse-timeout sets the kernel limit"
    );
    let gated = GatedBackend::new();
    let mut host = FuseTaskHost::start(HostSetup::new(gated.drive(RW)?, RW))?;
    let places = host.places();
    let mut file = File::create(host.path("slow.txt"))?;
    file.write_all(b"published by the retry")?;
    // A slow backend: the upload at fsync(2) outlasts the kernel's limit
    // (close(2) returns once the file is in the local write cache, G4 ML1).
    let gate = gated.gate(Gated::Upload);
    let closing = std::thread::spawn(move || file.sync_all());
    assert!(
        gate.entered_within(Duration::from_secs(30)),
        "the save never started its upload"
    );
    let closed = closing
        .join()
        .map_err(|_| io::Error::other("the closing thread panicked"))?;
    assert!(closed.is_err(), "fsync(2) fails when the kernel aborts");
    gate.release(false);
    let (result, reports) = host.end()?;
    let cause =
        format!("Kernel-Zeitlimit fs.fuse.default_request_timeout = {seconds} s überschritten");
    let detail = result.expect_err("the journal keeps the save");
    assert!(detail.contains(&cause), "{detail}");
    match last(&reports) {
        Some((MountStatus::Failed { detail }, Some(MountRecovery::Required))) => {
            assert!(detail.contains(&cause), "{detail}")
        }
        other => panic!("unexpected final report {other:?}"),
    }
    assert!(crate::vfs::Backend::stat(&*gated.inner, "/slow.txt").is_err());
    // Retry: a new host on the same places publishes the journal, in the
    // background since G4 ML1.
    let host = FuseTaskHost::start(HostSetup::new(gated.drive(RW)?, RW).at(&places))?;
    assert!(
        eventually(Duration::from_secs(30), || {
            crate::vfs::Backend::stat(&*gated.inner, "/slow.txt").is_ok()
        }),
        "the restarted host uploads the journal"
    );
    assert_eq!(gated.inner.bytes("/slow.txt"), b"published by the retry");
    unmount_cleanly(host)
}

/// The test binary itself, running only the child host on `places`.
fn spawn_child_host(places: &Places, stderr: Stdio) -> io::Result<Child> {
    let module = module_path!();
    let path = module.split_once("::").map_or(module, |(_, rest)| rest);
    Command::new(std::env::current_exe()?)
        .arg(format!("{path}::{CHILD_TEST}"))
        .args([
            "--exact",
            "--include-ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(SWITCH, "1")
        .env(CHILD_CACHE, places.beside("child-cache"))
        .env(CHILD_FOLDER, &places.folder)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn()
}

/// Child process of the two tests below: a host until it is killed.
#[test]
#[ignore = "child process of drive_task_fuse_host_kill_daemon_cleans_mount and drive_task_fuse_stale_mount_cleaned_before_remount"]
fn fuse_task_child_host_serves_until_killed() -> io::Result<()> {
    require_task_stage();
    let place = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, name.to_string()))
    };
    let places = Places::existing(place(CHILD_CACHE)?, place(CHILD_FOLDER)?)?;
    let (fixture, drive) = memory(RW)?;
    fixture.put("/child.txt", b"served by the child host");
    let host = FuseTaskHost::start(HostSetup::new(drive, RW).at(&places))?;
    // The parent kills this process; the limit only ends a forgotten child.
    std::thread::sleep(Duration::from_secs(600));
    drop(host);
    Ok(())
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_fuse_host_kill_daemon_cleans_mount() -> io::Result<()> {
    require_task_stage();
    let places = Places::new()?;
    let child = spawn_child_host(&places, Stdio::piped())?;
    let mut process = MountHostProcess::capture_linux(child, places.folder.clone())?;
    if !eventually(MOUNT_LIMIT, || mounted(&places.folder)) {
        let _ = process.kill();
        let exit = process.wait()?;
        panic!("the child host did not mount: {:?}", exit.detail);
    }
    assert_eq!(
        fs::read(places.folder.join("child.txt"))?,
        b"served by the child host"
    );
    // The host dies without unmounting; the daemon observes its end the
    // way `MountManager::tick` does (`try_wait`/`wait`).
    process.kill()?;
    let exit = process.wait()?;
    assert!(!exit.status.success());
    assert!(!mounted(&places.folder), "the dead mount stayed");
    assert_eq!(mode_of(&places.folder)?, 0o500);
    Ok(())
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_fuse_stale_mount_cleaned_before_remount() -> io::Result<()> {
    require_task_stage();
    let places = Places::new()?;
    let mut child = spawn_child_host(&places, Stdio::null())?;
    let mounted_in_time = eventually(MOUNT_LIMIT, || mounted(&places.folder));
    child.kill()?;
    child.wait()?;
    assert!(mounted_in_time, "the child host did not mount");
    // Killed without the daemon: the mount stays listed and is dead.
    assert!(mounted(&places.folder), "a killed host leaves its mount");
    let dead = fs::symlink_metadata(&places.folder).expect_err("a dead mount");
    assert_eq!(dead.raw_os_error(), Some(libc::ENOTCONN), "{dead}");
    // The next host on the folder removes it before it mounts.
    let (fixture, drive) = memory(RW)?;
    fixture.put("/fresh.txt", b"after the stale mount");
    let host = FuseTaskHost::start(HostSetup::new(drive, RW).at(&places))?;
    assert_eq!(fs::read(host.path("fresh.txt"))?, b"after the stale mount");
    unmount_cleanly(host)
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_fuse_mountpoint_locked_after_unmount() -> io::Result<()> {
    require_task_stage();
    let (fixture, drive) = memory(RW)?;
    fixture.put("/a.txt", b"a");
    let host = FuseTaskHost::start(HostSetup::new(drive, RW))?;
    let places = host.places();
    assert_eq!(host.names()?, ["a.txt"]);
    unmount_cleanly(host)?;
    assert_eq!(mode_of(&places.folder)?, 0o500);
    assert!(names_in(&places.folder)?.is_empty());
    // SAFETY: only reads the id of this process.
    if unsafe { libc::geteuid() } != 0 {
        // Nothing lands in the unmounted folder by mistake.
        let stray = fs::write(places.folder.join("stray.txt"), b"x").expect_err("locked");
        assert_eq!(stray.kind(), io::ErrorKind::PermissionDenied, "{stray}");
    }
    // The next mount opens the folder, its unmount locks it again.
    let host =
        FuseTaskHost::start(HostSetup::new(rooted(fixture.clone(), "/", RW)?, RW).at(&places))?;
    assert_eq!(host.names()?, ["a.txt"]);
    unmount_cleanly(host)?;
    assert_eq!(mode_of(&places.folder)?, 0o500);
    Ok(())
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_fuse_recovery_retries_pending_after_restart() -> io::Result<()> {
    require_task_stage();
    let fixture = OptimizationBackend::new();
    let drive = || rooted(fixture.clone(), "/", RW);
    fixture.fail_upload(true);
    let host = FuseTaskHost::start(HostSetup::new(drive()?, RW))?;
    let places = host.places();
    let mut file = File::create(host.path("pending.txt"))?;
    file.write_all(b"kept in the journal")?;
    // G4 ML1: close(2) returns once the file is in the local write cache; the
    // failing background upload keeps it in the journal.
    close_checked(file)?;
    let (result, reports) = host.finish(false)?;
    assert!(result.is_err(), "{result:?}");
    assert!(
        matches!(
            last(&reports),
            Some((MountStatus::Failed { .. }, Some(MountRecovery::Required)))
        ),
        "{reports:?}"
    );
    assert!(crate::vfs::Backend::stat(&*fixture, "/pending.txt").is_err());
    // After the restart the host resumes the upload in the background.
    fixture.fail_upload(false);
    let host = FuseTaskHost::start(HostSetup::new(drive()?, RW).at(&places))?;
    assert!(
        eventually(Duration::from_secs(30), || {
            crate::vfs::Backend::stat(&*fixture, "/pending.txt").is_ok()
        }),
        "the restarted host uploads the journal"
    );
    assert_eq!(fixture.bytes("/pending.txt"), b"kept in the journal");
    assert_eq!(fs::read(host.path("pending.txt"))?, b"kept in the journal");
    unmount_cleanly(host)
}
