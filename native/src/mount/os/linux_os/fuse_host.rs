//! The Linux drive host (`--mount-host`, DRV-M4): FUSE through `fuser` as
//! the platform of the shared host flow (`os/shared/host_session.rs`). The
//! order of the steps, the status reports, the update stop (V8) and the
//! teardown live there; this file owns the folder, the FUSE session, its
//! change delivery and its end.
//!
//! Session: `Config::default()` with two read loops (`n_threads = 2`,
//! `clone_fd`), owner-only access (`SessionACL::Owner`, so no
//! `user_allow_other` is needed), `nodev,nosuid,noatime`, type
//! `fuse.smartexplorer`, no `auto_unmount` (decision C2). The daemon removes
//! what a crashed host leaves (`daemon/os/linux_os/mount_cleanup.rs`).
//!
//! End of a session, decided in `destroy` before `fuser` lets go of the
//! mount: unmounted outside the host (a file manager's eject,
//! `se drive unmount`) ends like an unmount request; aborted while still
//! mounted (the kernel request timeout, `fusectl`) is a failure whose reason
//! names the kernel timeout when one is set; the journal keeps what was not
//! published, and Retry publishes it. The host's own unmount is lazy; open
//! descriptors that keep the connection alive are cut through `fusectl`
//! after a short grace, and their files are then published like at a Dokany
//! unmount.
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use fuser::{Config, MountOption, Notifier, Session, SessionACL, SessionUnmounter};

use super::super::shared::change_pump::ChangePump;
use super::super::shared::host_platform::{
    HostLease, HostPlatform, HostStepError, LeaseFn, VolumeLocation, VolumePoll,
};
use super::super::shared::host_session::{self, HostExit, HostInputs, UpdateStopCheck};
use super::bookmarks::{self, BookmarkEntry};
use super::cache_lease::CacheLease;
use super::cache_space::CacheDiskSpace;
use super::fuse_fs::{FuseFs, FuseShared, SessionEnd};
use super::fuse_notify::FuseSink;
use super::fuse_runtime::RuntimeProbe;
use super::fuse_workers::RefreshWorker;
use super::mount_point::{self, PlaceKind};
use crate::daemon::{connect_mount_host, MountHostConfig};
use crate::mount::{CacheSpaceProbe, MountEngine, MountFilesystem, MountId, PathDialect};

/// Read loops of `fuser` (decision C2).
const READ_LOOPS: usize = 2;
/// How long a lazily unmounted session may serve open descriptors.
const UNMOUNT_GRACE: Duration = Duration::from_secs(2);
/// How long an aborted session may take to end.
const ABORT_GRACE: Duration = Duration::from_secs(10);
/// How long requests that are still running may finish.
const DRAIN_GRACE: Duration = Duration::from_secs(5);
/// The one-use token the daemon hands to its host
/// (`daemon/os/linux_os/mount_process.rs`).
const HOST_TOKEN_ENV: &str = "SMART_EXPLORER_MOUNT_TOKEN";
const ENDED: &str = "the FUSE session ended without a requested unmount";

pub(crate) fn run_mount_host(id: MountId) -> Result<HostExit, String> {
    // V8: the baseline is the marker present when the process starts.
    let update_stop = UpdateStopCheck::arm();
    // Without the daemon's token nothing below can succeed; checking first
    // leaves the process environment as it is.
    if std::env::var_os(HOST_TOKEN_ENV).is_none() {
        return Err("Laufwerk-Host-Token fehlt".into());
    }
    let session = Arc::new(connect_mount_host(id)?);
    let inputs = HostInputs::from_session(&session);
    let mut platform = FuseHost::default();
    // DRV-M6: the device tree has its own host flow over the same platform.
    if session.config.device_tree {
        return host_session::devices::run(inputs, &session, &mut platform, &update_stop);
    }
    host_session::run(inputs, &*session, &mut platform, &update_stop)
}

/// FUSE as platform of the shared host flow.
#[derive(Default)]
pub(crate) struct FuseHost {
    place: Option<Place>,
    shared: Option<Arc<FuseShared>>,
    session: Option<Running>,
    refresh: Option<RefreshWorker>,
    pump: ChangePump,
    cause: Option<String>,
}

struct Place {
    /// The folder as configured; reported as `MountedAt`.
    configured: PathBuf,
    /// The folder as `mountinfo` lists it.
    canonical: PathBuf,
    kind: PlaceKind,
    /// The places of the file managers (DRV-M5) name the drive.
    id: MountId,
    label: String,
}

struct Running {
    thread: JoinHandle<io::Result<()>>,
    notifier: Notifier,
    unmounter: SessionUnmounter,
    /// The `fusectl` number, read right after the mount.
    connection: Option<u32>,
}

impl HostPlatform for FuseHost {
    fn path_dialect(&self) -> PathDialect {
        PathDialect::Posix
    }

    fn acquire_lease(&mut self, spool_root: &Path, id: &MountId) -> io::Result<HostLease> {
        CacheLease::acquire(spool_root, id).map(|lease| Box::new(lease) as HostLease)
    }

    fn lease_fn(&self) -> Option<LeaseFn> {
        Some(Arc::new(|spool_root: &Path, id: &MountId| {
            CacheLease::acquire(spool_root, id).map(|lease| Box::new(lease) as HostLease)
        }))
    }

    fn space_probe(&mut self, spool_root: &Path) -> io::Result<Arc<dyn CacheSpaceProbe>> {
        CacheDiskSpace::new(spool_root).map(|space| Arc::new(space) as Arc<dyn CacheSpaceProbe>)
    }

    fn select_runtime(
        &mut self,
        _spool_root: &Path,
        _config: &MountHostConfig,
    ) -> Result<(), String> {
        let report = super::fuse_runtime::report(&RuntimeProbe::system(false));
        if report.ready {
            Ok(())
        } else {
            Err(report.problems.join("; "))
        }
    }

    fn prepare_volume(
        &mut self,
        engine: &Arc<MountEngine>,
        config: &MountHostConfig,
        spool_root: &Path,
    ) -> Result<(), HostStepError> {
        let filesystem: Arc<dyn MountFilesystem> = engine.clone();
        self.prepare_filesystem(filesystem, config, spool_root)
    }

    fn prepare_filesystem(
        &mut self,
        filesystem: Arc<dyn MountFilesystem>,
        config: &MountHostConfig,
        spool_root: &Path,
    ) -> Result<(), HostStepError> {
        let configured = location_of(config).map_err(|error| {
            HostStepError::Plain(format!("the drive has no mount folder: {error}"))
        })?;
        let kind = mount_point::place_kind(&configured);
        let canonical = mount_point::prepare(&configured, kind)
            .and_then(|()| mount_point::canonical(&configured))
            .map_err(|error| HostStepError::Plain(format!("mount folder: {error}")))?;
        let read_only = filesystem.volume_facts().read_only;
        self.shared = Some(FuseShared::new(
            filesystem,
            read_only,
            spool_root.to_path_buf(),
            canonical.clone(),
        ));
        self.place = Some(Place {
            configured,
            canonical,
            kind,
            id: config.id.clone(),
            label: config.label.clone(),
        });
        Ok(())
    }

    fn start_volume(&mut self) -> Result<(), String> {
        let (Some(shared), Some(place)) = (self.shared.as_ref(), self.place.as_ref()) else {
            return Err("FUSE could not mount the drive: the volume was not prepared".into());
        };
        let mut config = Config::default();
        config.mount_options = vec![
            MountOption::FSName(mount_point::FUSE_SUBTYPE.into()),
            MountOption::Subtype(mount_point::FUSE_SUBTYPE.into()),
            MountOption::NoDev,
            MountOption::NoSuid,
            MountOption::NoAtime,
            if shared.read_only {
                MountOption::RO
            } else {
                MountOption::RW
            },
        ];
        config.acl = SessionACL::Owner;
        config.n_threads = Some(READ_LOOPS);
        config.clone_fd = true;
        mount_point::open_for_mount(&place.configured, place.kind)
            .map_err(|error| format!("FUSE could not open the mount folder: {error}"))?;
        let mut session =
            match Session::new(FuseFs::new(Arc::clone(shared)), &place.configured, &config) {
                Ok(session) => session,
                Err(error) => {
                    let _ = mount_point::lock(&place.configured, place.kind);
                    return Err(format!("FUSE could not mount the drive: {error}"));
                }
            };
        let notifier = session.notifier();
        let unmounter = session.unmount_callable();
        let connection = super::fuse_abort::connection_at(&place.canonical);
        let thread = std::thread::Builder::new()
            .name("fuse-session".into())
            .spawn(move || session.run())
            .map_err(|error| format!("FUSE session thread: {error}"))?;
        self.session = Some(Running {
            thread,
            notifier,
            unmounter,
            connection,
        });
        // DRV-M5: a failed entry never fails the mount.
        if let Err(error) = bookmarks::add(&BookmarkEntry {
            id: place.id.to_string(),
            folder: place.configured.clone(),
            label: place.label.clone(),
        }) {
            eprintln!("Dateimanager-Eintrag: {error}");
        }
        Ok(())
    }

    fn start_background(&mut self) -> io::Result<()> {
        let shared = self
            .shared
            .as_ref()
            .ok_or_else(|| io::Error::other("the FUSE volume was not prepared"))?;
        self.refresh = Some(RefreshWorker::start(Arc::clone(&shared.filesystem))?);
        Ok(())
    }

    fn location(&self) -> io::Result<VolumeLocation> {
        self.place
            .as_ref()
            .map(|place| VolumeLocation::Directory(place.configured.clone()))
            .ok_or_else(|| io::Error::other("the FUSE volume has no folder"))
    }

    fn stop_requested(&self) -> bool {
        self.shared
            .as_ref()
            .is_some_and(|shared| shared.stop_requested())
    }

    fn poll_volume(&mut self) -> VolumePoll {
        let Some(running) = self.session.as_ref() else {
            return VolumePoll::Ended(ENDED);
        };
        if !running.thread.is_finished() {
            return VolumePoll::Running;
        }
        let end = self.shared.as_ref().and_then(|shared| shared.session_end());
        match end {
            Some(SessionEnd::Unmounted) => VolumePoll::Unmounted,
            Some(SessionEnd::Aborted) => {
                self.cause = Some(match super::fuse_runtime::kernel_request_timeout() {
                    Some(seconds) => format!(
                        "Kernel-Zeitlimit fs.fuse.default_request_timeout = {seconds} s überschritten"
                    ),
                    None => "die FUSE-Verbindung wurde abgebrochen".into(),
                });
                VolumePoll::Ended("the FUSE connection was aborted")
            }
            Some(SessionEnd::Requested) | None => VolumePoll::Ended(ENDED),
        }
    }

    fn deliver_changes(&mut self) -> io::Result<()> {
        let (Some(shared), Some(running)) = (self.shared.as_ref(), self.session.as_ref()) else {
            return Ok(());
        };
        if running.thread.is_finished() {
            return Ok(());
        }
        let mut sink = FuseSink {
            notifier: &running.notifier,
            inodes: &shared.inodes,
        };
        self.pump.deliver(&*shared.filesystem, &mut sink)
    }

    fn shutdown(&mut self) -> bool {
        if let Some(refresh) = self.refresh.as_ref() {
            refresh.request_stop();
        }
        let mut ended = true;
        let mut requested = false;
        if let (Some(running), Some(shared)) = (self.session.as_mut(), self.shared.as_ref()) {
            if !running.thread.is_finished() {
                requested = true;
                shared.unmounting.store(true, Ordering::Release);
                // Lazy: the name goes at once, open descriptors keep the
                // connection until they close or the grace ends.
                let _ = running.unmounter.unmount();
                if !finished_within(&running.thread, UNMOUNT_GRACE) {
                    if let Some(connection) = running.connection {
                        let _ = super::fuse_abort::abort_connection(connection);
                    }
                    ended = finished_within(&running.thread, ABORT_GRACE);
                }
            }
        }
        if let Some(shared) = self.shared.as_ref() {
            let _ = shared.workers.drain(DRAIN_GRACE);
            // After an abort or an outside unmount nothing waits on a
            // backend that may hang; the journal keeps what is pending.
            super::fuse_ops_io::close_all(shared, requested);
            shared.workers.close();
        }
        if let Some(mut refresh) = self.refresh.take() {
            refresh.join();
        }
        if let Some(place) = self.place.as_ref() {
            // DRV-M5: the places go right after the unmount, before the
            // folder is locked.
            if let Err(error) = bookmarks::remove(&place.configured) {
                eprintln!("Dateimanager-Eintrag: {error}");
            }
            let _ = mount_point::lock(&place.configured, place.kind);
        }
        ended
    }

    fn release_volume(&mut self) {
        if let Some(running) = self.session.take() {
            // A session that could not be ended stays with the process.
            if running.thread.is_finished() {
                let _ = running.thread.join();
            }
        }
        self.refresh = None;
        self.shared = None;
    }

    fn finish_runtime(&mut self, _completed: bool) {}

    fn failure_cause(&self) -> Option<String> {
        self.cause.clone()
    }
}

/// The folder of the drive: the daemon resolves it (`MountHostConfig::
/// location`); without one, the default below `~/SmartExplorer`.
fn location_of(config: &MountHostConfig) -> io::Result<PathBuf> {
    match &config.location {
        Some(location) => Ok(location.path().to_path_buf()),
        None => mount_point::drives_root()
            .map(|root| root.join(mount_point::folder_name(&config.label))),
    }
}

fn finished_within(thread: &JoinHandle<io::Result<()>>, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while !thread.is_finished() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}
