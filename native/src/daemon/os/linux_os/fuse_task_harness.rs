//! Harness of the FUSE drive signals (DRV-M4): the production host flow
//! (`host_session::run` with `FuseHost`) on a thread of the test process,
//! over a backend behind the daemon's authorized root, with a channel as
//! the daemon (reports) and a flag as its unmount request. The tests touch
//! the mounted folder through ordinary file calls and programs.
//!
//! Every test that mounts needs `fusermount3`, `/dev/fuse` and the switch
//! `SMART_EXPLORER_FUSE_TASK=1`; they are `#[ignore]` and run in the task
//! stage `linux-root: drive-fuse` with `--include-ignored` and one test
//! thread, as the runner user (never as root: `fusermount3` is the path a
//! user takes).
use std::fs::File;
use std::os::fd::IntoRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use std::{fs, io};

use crate::daemon::{MountHostConfig, UpdateStopBaseline, UPDATE_STOP_POLL};
use crate::mount::os::linux_os::fuse_host::FuseHost;
use crate::mount::os::linux_os::{fuse_abort, mount_point};
use crate::mount::os::shared::host_session::{
    self, HostControl, HostExit, HostInputs, UpdateStopCheck,
};
use crate::mount::{
    BackendRoot, DriveSelection, MountCachePolicy, MountConfig, MountId, MountMode, MountRecovery,
    MountSource, MountStatus, PathDialect,
};
use crate::vfs::BackendHandle;

pub(super) const SWITCH: &str = "SMART_EXPLORER_FUSE_TASK";
pub(super) const MOUNT_LIMIT: Duration = Duration::from_secs(60);
const END_LIMIT: Duration = Duration::from_secs(180);

pub(super) type Report = (MountStatus, Option<MountRecovery>);
pub(super) type Ended = (Result<HostExit, String>, Vec<Report>);

/// A test that mounts fails loudly outside its task stage.
pub(super) fn require_task_stage() {
    assert_eq!(
        std::env::var(SWITCH).as_deref(),
        Ok("1"),
        "runs only in the stage linux-root: drive-fuse of rechte-desktop-tresore-task (fusermount3, /dev/fuse)"
    );
}

/// An se drive is mounted at `folder` (live or dead).
pub(super) fn mounted(folder: &Path) -> bool {
    let canonical = mount_point::canonical(folder).unwrap_or_else(|_| folder.to_path_buf());
    fuse_abort::mount_at(&canonical).is_some_and(|entry| entry.fstype == mount_point::FUSE_TYPE)
}

/// Polls `condition` every 50 ms up to `limit`.
pub(super) fn eventually(limit: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `close(2)` with its result, which dropping a `File` discards: the
/// publication of a save reports through it.
pub(super) fn close_checked(file: File) -> io::Result<()> {
    let descriptor = file.into_raw_fd();
    // SAFETY: the descriptor comes from `into_raw_fd` and is closed once.
    if unsafe { libc::close(descriptor) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Mount folder, cache and drive id, kept across hosts (Retry, remount,
/// a stale mount); the temporary folder goes with the last copy.
#[derive(Clone)]
pub(super) struct Places {
    pub(super) id: MountId,
    pub(super) cache: PathBuf,
    pub(super) folder: PathBuf,
    base: Option<Arc<tempfile::TempDir>>,
}

impl Places {
    pub(super) fn new() -> io::Result<Self> {
        let base = tempfile::tempdir()?;
        Ok(Self {
            id: MountId::new_random()?,
            cache: base.path().join("cache"),
            folder: base.path().join("mnt"),
            base: Some(Arc::new(base)),
        })
    }

    /// Places another process owns (the child host).
    pub(super) fn existing(cache: PathBuf, folder: PathBuf) -> io::Result<Self> {
        Ok(Self {
            id: MountId::new_random()?,
            cache,
            folder,
            base: None,
        })
    }

    /// A path below the temporary folder, outside cache and mount folder.
    pub(super) fn beside(&self, name: &str) -> PathBuf {
        match &self.base {
            Some(base) => base.path().join(name),
            None => self.cache.with_file_name(name),
        }
    }
}

struct ChannelControl {
    reports: mpsc::Sender<Report>,
    stop: Arc<AtomicBool>,
}

impl HostControl for ChannelControl {
    fn report(&self, status: MountStatus, recovery: Option<MountRecovery>) -> Result<(), String> {
        let _ = self.reports.send((status, recovery));
        Ok(())
    }

    fn wait_for_stop(&self, timeout: Duration) -> bool {
        std::thread::sleep(timeout);
        self.stop.load(Ordering::Acquire)
    }
}

/// What a test host serves and where.
pub(super) struct HostSetup {
    pub(super) backend: BackendHandle,
    pub(super) mode: MountMode,
    /// The places of an earlier host; fresh ones otherwise.
    pub(super) places: Option<Places>,
    /// Idle clean cache in MiB (500 when not set).
    pub(super) cache_mib: Option<u32>,
}

impl HostSetup {
    pub(super) fn new(backend: BackendHandle, mode: MountMode) -> Self {
        Self {
            backend,
            mode,
            places: None,
            cache_mib: None,
        }
    }

    pub(super) fn at(mut self, places: &Places) -> Self {
        self.places = Some(places.clone());
        self
    }
}

/// One production FUSE host on its own thread.
pub(super) struct FuseTaskHost {
    pub(super) folder: PathBuf,
    places: Places,
    reports: mpsc::Receiver<Report>,
    done: mpsc::Receiver<Result<HostExit, String>>,
    stop: Arc<AtomicBool>,
    marker: Arc<Mutex<Option<SystemTime>>>,
    ended: bool,
}

impl FuseTaskHost {
    /// Starts the host and waits for its `MountedAt` report.
    pub(super) fn start(setup: HostSetup) -> io::Result<Self> {
        let places = match setup.places {
            Some(places) => places,
            None => Places::new()?,
        };
        let mut config = MountConfig::new(
            places.id.clone(),
            MountSource::SavedRemote {
                account: "sftp:fuse-task".into(),
                root: BackendRoot::parse("/")?,
            },
            DriveSelection::Automatic,
            setup.mode,
            "FUSE task",
        )?
        .with_location_directory(Some(places.folder.clone()))?;
        if let Some(mib) = setup.cache_mib {
            config = config.with_cache_policy(MountCachePolicy::new(mib)?);
        }
        let mut host_config = MountHostConfig::from(&config);
        host_config.path_dialect = PathDialect::Posix;
        host_config.location = config.location.clone();
        let inputs = HostInputs {
            config: host_config,
            backend: setup.backend,
            cache_root: places.cache.clone(),
        };
        let (report_send, reports) = mpsc::channel();
        let (done_send, done) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let marker = Arc::new(Mutex::new(None));
        let thread_stop = Arc::clone(&stop);
        let source = Arc::clone(&marker);
        std::thread::Builder::new()
            .name("fuse-task-host".into())
            .spawn(move || {
                let baseline = UpdateStopBaseline::with_source(Arc::new(move || {
                    *source.lock().unwrap_or_else(|error| error.into_inner())
                }));
                let check = UpdateStopCheck::new(baseline, UPDATE_STOP_POLL);
                let control = ChannelControl {
                    reports: report_send,
                    stop: thread_stop,
                };
                let mut platform = FuseHost::default();
                let _ = done_send.send(host_session::run(inputs, &control, &mut platform, &check));
            })?;
        let mut host = Self {
            folder: places.folder.clone(),
            places,
            reports,
            done,
            stop,
            marker,
            ended: false,
        };
        if let Err(error) = host.await_mounted() {
            // A host that failed to mount has ended or ends now.
            host.stop.store(true, Ordering::Release);
            let _ = host.done.recv_timeout(END_LIMIT);
            host.ended = true;
            return Err(error);
        }
        Ok(host)
    }

    fn await_mounted(&self) -> io::Result<()> {
        let deadline = Instant::now() + MOUNT_LIMIT;
        loop {
            match self
                .reports
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok((MountStatus::MountedAt { path }, _)) if path == self.folder => return Ok(()),
                Ok((MountStatus::Failed { detail }, _))
                | Ok((MountStatus::RuntimeUnavailable { detail }, _)) => {
                    return Err(io::Error::other(format!("host did not mount: {detail}")))
                }
                Ok(_) => {}
                Err(error) => return Err(io::Error::other(format!("no MountedAt: {error}"))),
            }
        }
    }

    /// The places, for a later host on the same folder and cache.
    pub(super) fn places(&self) -> Places {
        self.places.clone()
    }

    /// Ends the host with an update stop or an unmount request; returns its
    /// result and every report since `MountedAt`.
    pub(super) fn finish(mut self, update_stop: bool) -> io::Result<Ended> {
        if update_stop {
            *self
                .marker
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(SystemTime::now());
        } else {
            self.stop.store(true, Ordering::Release);
        }
        self.end()
    }

    /// Waits for a host that ends on its own (outside unmount, abort).
    pub(super) fn end(&mut self) -> io::Result<Ended> {
        let result = self
            .done
            .recv_timeout(END_LIMIT)
            .map_err(|error| io::Error::other(format!("the host did not end: {error}")))?;
        self.ended = true;
        Ok((result, self.reports.try_iter().collect()))
    }

    /// The path of `name` in the mounted folder.
    pub(super) fn path(&self, name: &str) -> PathBuf {
        self.folder.join(name)
    }

    /// Names in the mounted root, sorted.
    pub(super) fn names(&self) -> io::Result<Vec<String>> {
        names_in(&self.folder)
    }
}

impl Drop for FuseTaskHost {
    fn drop(&mut self) {
        // A failed assertion must not leave a mounted drive behind.
        if !self.ended {
            self.stop.store(true, Ordering::Release);
            let _ = self.done.recv_timeout(END_LIMIT);
        }
    }
}

/// Names in `folder`, sorted.
pub(super) fn names_in(folder: &Path) -> io::Result<Vec<String>> {
    let mut names = fs::read_dir(folder)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<io::Result<Vec<_>>>()?;
    names.sort();
    Ok(names)
}

/// The last report, which every end of a host leaves.
pub(super) fn last(reports: &[Report]) -> Option<&Report> {
    reports.last()
}

/// The report of a host that ended cleanly.
pub(super) fn clean_end() -> Report {
    (MountStatus::Unmounted, Some(MountRecovery::Clean))
}

/// Runs a program and returns its exit code and output.
pub(super) fn program(command: &mut std::process::Command) -> io::Result<(i32, String)> {
    let output = command.output()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok((output.status.code().unwrap_or(-1), text))
}

/// A program on `PATH`.
pub(super) fn tool(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

/// The content of `path` on a backend.
pub(super) fn remote_bytes(backend: &BackendHandle, path: &str) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    io::Read::read_to_end(&mut backend.open_read(path)?, &mut bytes)?;
    Ok(bytes)
}

/// Ends the host with an unmount request and checks the clean end: the
/// final report, and no mount left at the folder.
pub(super) fn unmount_cleanly(host: FuseTaskHost) -> io::Result<()> {
    let folder = host.folder.clone();
    let (result, reports) = host.finish(false)?;
    assert_eq!(result, Ok(HostExit::Stopped));
    assert_eq!(last(&reports), Some(&clean_end()));
    assert!(!mounted(&folder), "the drive stayed mounted");
    Ok(())
}

/// Removes a test folder on a server, files first; a link is removed as
/// a file, never followed.
pub(super) fn remove_remote_tree(raw: &BackendHandle, folder: &str) -> io::Result<()> {
    for entry in raw.list_dir(folder)? {
        let path = format!("{folder}/{}", entry.name);
        if entry.is_dir && !entry.is_symlink {
            remove_remote_tree(raw, &path)?;
        } else {
            raw.remove_file(&path)?;
        }
    }
    raw.remove_dir(folder)
}
