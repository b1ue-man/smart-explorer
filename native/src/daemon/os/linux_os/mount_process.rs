//! The drive host process under Linux (DRV-M4): the current executable
//! with `--mount-host <id>` and the one-use environment the host takes
//! (`mount_client.rs`). The daemon's environment is kept (the host needs
//! `PATH`, `HOME`, `XDG_RUNTIME_DIR`); `FUSERMOUNT_PATH` names the checked
//! setuid helper. The host receives `PR_SET_PDEATHSIG(SIGTERM)`: it never
//! outlives the daemon that supervises it (decision C2). Because the kernel
//! sends that signal when the forking *thread* ends, every host is forked by
//! one keeper thread that lives as long as the daemon. A FUSE mount a host
//! leaves behind is removed when its end is observed (`mount_cleanup.rs`).
//! Android starts no drive host (it offers folders through its documents
//! provider, DRV-M8).
#[cfg(target_os = "linux")]
use std::io;
#[cfg(target_os = "linux")]
use std::net::SocketAddr;
#[cfg(target_os = "linux")]
use std::path::Path;

#[cfg(target_os = "linux")]
use super::mount_host_process::MountHostProcess;

pub(super) const MOUNT_TOKEN_ENV: &str = "SMART_EXPLORER_MOUNT_TOKEN";
pub(super) const MOUNT_IPC_ADDR_ENV: &str = "SMART_EXPLORER_MOUNT_IPC_ADDR";
pub(super) const MOUNT_CACHE_DIR_ENV: &str = "SMART_EXPLORER_MOUNT_CACHE_DIR";

#[cfg(target_os = "linux")]
#[path = "mount_cleanup.rs"]
mod mount_cleanup;
#[cfg(target_os = "linux")]
pub(super) use mount_cleanup::clean_after_host;

#[cfg(all(test, target_os = "linux"))]
#[path = "fuse_task_backends.rs"]
mod fuse_task_backends;
#[cfg(all(test, target_os = "linux"))]
#[path = "fuse_task_daemon_tests.rs"]
mod fuse_task_daemon_tests;
#[cfg(all(test, target_os = "linux"))]
#[path = "fuse_task_harness.rs"]
mod fuse_task_harness;
#[cfg(all(test, target_os = "linux"))]
#[path = "fuse_task_life_tests.rs"]
mod fuse_task_life_tests;
#[cfg(all(test, target_os = "linux"))]
#[path = "fuse_task_rules_tests.rs"]
mod fuse_task_rules_tests;
#[cfg(all(test, target_os = "linux"))]
#[path = "fuse_task_tests.rs"]
mod fuse_task_tests;

/// Starts the host of `mount_id` for the folder `mount_point`.
#[cfg(target_os = "linux")]
pub(super) fn spawn(
    mount_id: &crate::mount::MountId,
    launch_token: &str,
    ipc_addr: SocketAddr,
    cache_root: &Path,
    mount_point: &Path,
) -> io::Result<MountHostProcess> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--mount-host")
        .arg(mount_id.as_str())
        .env(MOUNT_TOKEN_ENV, launch_token)
        .env(MOUNT_IPC_ADDR_ENV, ipc_addr.to_string())
        .env(MOUNT_CACHE_DIR_ENV, cache_root)
        .env(
            "FUSERMOUNT_PATH",
            crate::mount::os::linux_os::fuse_runtime::helper_program(),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let daemon = std::process::id();
    // SAFETY: the closure runs between fork and exec and only calls the
    // async-signal-safe `prctl` and `getppid`.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM as libc::c_ulong) != 0 {
                return Err(io::Error::last_os_error());
            }
            // The daemon may have ended before the signal was armed.
            if u32::try_from(libc::getppid()).ok() != Some(daemon) {
                return Err(io::Error::from_raw_os_error(libc::ESRCH));
            }
            Ok(())
        });
    }
    let child = keeper::spawn(command)
        .map_err(|error| io::Error::new(error.kind(), format!("Laufwerk-Host starten: {error}")))?;
    MountHostProcess::capture_linux(child, mount_point.to_path_buf())
}

#[cfg(target_os = "linux")]
mod keeper {
    use std::io;
    use std::process::{Child, Command};
    use std::sync::mpsc::{self, Sender};
    use std::sync::Mutex;

    type Job = (Command, Sender<io::Result<Child>>);

    static KEEPER: Mutex<Option<Sender<Job>>> = Mutex::new(None);

    /// Forks `command` on the keeper thread; a keeper that ended is
    /// replaced.
    pub(super) fn spawn(command: Command) -> io::Result<Child> {
        let (reply, answer) = mpsc::channel();
        {
            let mut keeper = KEEPER
                .lock()
                .map_err(|_| io::Error::other("the mount host keeper is unavailable"))?;
            let job: Job = (command, reply);
            let job = match keeper.as_ref() {
                Some(sender) => match sender.send(job) {
                    Ok(()) => None,
                    Err(mpsc::SendError(job)) => Some(job),
                },
                None => Some(job),
            };
            if let Some(job) = job {
                let (send, receive) = mpsc::channel::<Job>();
                std::thread::Builder::new()
                    .name("mount-host-keeper".into())
                    .spawn(move || {
                        for (mut command, reply) in receive {
                            let _ = reply.send(command.spawn());
                        }
                    })?;
                send.send(job)
                    .map_err(|_| io::Error::other("the mount host keeper ended"))?;
                *keeper = Some(send);
            }
        }
        answer
            .recv()
            .map_err(|_| io::Error::other("the mount host keeper ended"))?
    }
}
