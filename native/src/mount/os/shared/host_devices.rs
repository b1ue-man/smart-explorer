//! The host of the device tree (DRV-M6, U17-11 and U17-19): the flow of
//! `host_session.rs` over a `DeviceTree` instead of one engine.
//!
//! The daemon hands this host a placeholder backend (the handshake stays
//! the one of every drive); the tree reaches the devices through further
//! connections of its own (`MountHostSession::child_backend`,
//! `child_roots`). Nothing in the start waits for a device: the tree opens a
//! child at the first access, and the device list arrives from a watcher
//! thread. The control loop of the shared flow runs unchanged; a `tick` of
//! the tree rides on every wait of the loop.
use std::any::Any;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use crate::daemon::{MountHostConfig, MountHostSession};
use crate::mount::{
    prepare_spool_root, BackendRoot, ChildConnection, ChildLink, ChildRoot, DeviceRow, DeviceTree,
    MountFilesystem, MountId, MountRecovery, MountStatus, PeerMountTarget, TreeConfig,
};

use super::super::host_platform::{HostPlatform, HostStepError, LeaseFn};
use super::{
    finish, report_failure, report_failure_with_recovery, run_until_stopped, Ended, HostControl,
    HostExit, HostInputs, LockAnswer, LockOrder, SpoolKeyAnswer, UpdateStopCheck,
};

/// After a failed request the watcher waits this long before it asks again,
/// so a daemon that is gone is not asked in a tight loop.
const WATCH_RETRY: Duration = Duration::from_secs(5);
const WATCH_STEP: Duration = Duration::from_millis(250);

/// Where the device list comes from: the daemon in production.
pub(crate) trait DeviceFeed: Send + Sync {
    /// Waits for a list newer than revision `since` and returns it with its
    /// own revision; a list that did not change may come back unchanged
    /// after the longest wait.
    fn watch(&self, since: u64) -> Result<(u64, Vec<DeviceRow>), String>;
}

impl DeviceFeed for MountHostSession {
    fn watch(&self, since: u64) -> Result<(u64, Vec<DeviceRow>), String> {
        self.device_watch(since)
    }
}

/// Runs the host of the device tree to its end. The result is that of
/// `host_session::run`.
pub(crate) fn run(
    inputs: HostInputs,
    session: &Arc<MountHostSession>,
    platform: &mut dyn HostPlatform,
    update_stop: &UpdateStopCheck,
) -> Result<HostExit, String> {
    let linked = Arc::clone(session);
    run_with(
        inputs,
        &**session,
        move |lease| Arc::new(HostChildLink::new(linked, lease)) as Arc<dyn ChildLink>,
        Arc::clone(session) as Arc<dyn DeviceFeed>,
        platform,
        update_stop,
    )
}

/// `run` with the daemon replaced by its parts: the control, the link to the
/// devices (made from the lease function of the platform) and the feed of
/// the device list.
pub(crate) fn run_with(
    inputs: HostInputs,
    control: &dyn HostControl,
    link: impl FnOnce(LeaseFn) -> Arc<dyn ChildLink>,
    feed: Arc<dyn DeviceFeed>,
    platform: &mut dyn HostPlatform,
    update_stop: &UpdateStopCheck,
) -> Result<HostExit, String> {
    let HostInputs {
        config, cache_root, ..
    } = inputs;
    let spool_root = prepare_spool_root(&cache_root).map_err(|_| {
        report_failure(control, "the isolated mount cache directory is unavailable")
    })?;
    let lease = platform
        .acquire_lease(&spool_root, &config.id)
        .map_err(|_| {
            report_failure_with_recovery(
                control,
                "the mounted-drive cache is already in use or its lease is unsafe",
                MountRecovery::Unknown,
            )
        })?;
    let ended = run_leased(
        &config,
        &spool_root,
        control,
        link,
        feed,
        platform,
        update_stop,
    );
    platform.release_volume();
    platform.finish_runtime(ended.teardown.is_ok());
    drop(lease);
    match ended.exit {
        HostExit::UpdateStop => Ok(HostExit::UpdateStop),
        HostExit::Stopped => ended.teardown.map(|()| HostExit::Stopped),
    }
}

fn run_leased(
    config: &MountHostConfig,
    spool_root: &Path,
    control: &dyn HostControl,
    link: impl FnOnce(LeaseFn) -> Arc<dyn ChildLink>,
    feed: Arc<dyn DeviceFeed>,
    platform: &mut dyn HostPlatform,
    update_stop: &UpdateStopCheck,
) -> Ended {
    let dialect = platform.path_dialect();
    if config.path_dialect != dialect {
        return Ended::failed(report_failure(
            control,
            "the drive host serves other name rules than the configuration requests",
        ));
    }
    let space = match platform.space_probe(spool_root) {
        Ok(space) => space,
        Err(error) => {
            return Ended::failed(report_failure(
                control,
                &format!("cache space probe: {error}"),
            ))
        }
    };
    let Some(lease) = platform.lease_fn() else {
        return Ended::failed(report_failure(
            control,
            "this drive host cannot serve the device tree",
        ));
    };
    let link = link(lease);
    let tree = match DeviceTree::new(
        TreeConfig {
            id: config.id.clone(),
            metadata: config.metadata,
            cache: config.cache,
            space,
        },
        link,
        spool_root,
        dialect,
    ) {
        Ok(tree) => tree,
        Err(_) => {
            return Ended::failed(report_failure_with_recovery(
                control,
                "the device tree cache could not be recovered safely",
                MountRecovery::Unknown,
            ))
        }
    };
    if let Err(error) = control.report(MountStatus::Mounting, Some(recovery_of(&tree))) {
        return Ended::failed(error);
    }
    if let Err(detail) = platform.select_runtime(spool_root, config) {
        let _ = control.report(
            MountStatus::RuntimeUnavailable {
                detail: detail.clone(),
            },
            None,
        );
        return Ended::failed(detail);
    }
    let filesystem: Arc<dyn MountFilesystem> = tree.clone();
    if let Err(error) = platform.prepare_filesystem(filesystem, config, spool_root) {
        let (HostStepError::Plain(detail) | HostStepError::Engine(detail)) = error;
        return Ended::failed(report_failure_with_recovery(
            control,
            &detail,
            recovery_of(&tree),
        ));
    }
    // Recovery ownership is armed before the frontend dispatches its first
    // callback, exactly like the flow of an engine drive.
    if let Err(error) = control.report(MountStatus::Mounting, Some(MountRecovery::Unknown)) {
        return Ended::failed(error);
    }
    if let Err(detail) = platform.start_volume() {
        return Ended::failed(report_failure_with_recovery(
            control,
            &detail,
            recovery_of(&tree),
        ));
    }
    let stop = Arc::new(AtomicBool::new(false));
    start_watcher(feed, &tree, &stop);
    let ticking = Ticking {
        inner: control,
        tree: Arc::clone(&tree),
    };
    let ended = match platform.start_background() {
        Ok(()) => run_until_stopped(platform, &ticking, &*tree, update_stop),
        Err(error) => {
            let detail = format!("mounted-drive metadata refresh could not start: {error}");
            finish(platform, &ticking, &*tree, Some(&detail), HostExit::Stopped)
        }
    };
    stop.store(true, Ordering::Release);
    ended
}

/// What the journals of the children say: clean, or in need of a retry.
fn recovery_of(tree: &DeviceTree) -> MountRecovery {
    match tree.dirty_entries() {
        Ok(entries) if entries.is_empty() => MountRecovery::Clean,
        Ok(_) => MountRecovery::Required,
        Err(_) => MountRecovery::Unknown,
    }
}

/// The control of the daemon with a `tick` of the tree on every wait of
/// the control loop (every 250 ms).
struct Ticking<'a> {
    inner: &'a dyn HostControl,
    tree: Arc<DeviceTree>,
}

impl HostControl for Ticking<'_> {
    fn report(&self, status: MountStatus, recovery: Option<MountRecovery>) -> Result<(), String> {
        self.inner.report(status, recovery)
    }

    fn wait_for_stop(&self, timeout: Duration) -> bool {
        self.tree.tick();
        self.inner.wait_for_stop(timeout)
    }

    fn spool_key(&self) -> SpoolKeyAnswer {
        self.inner.spool_key()
    }

    fn lock_order(&self) -> Option<LockOrder> {
        self.inner.lock_order()
    }

    fn answer_lock(&self, answer: LockAnswer) {
        self.inner.answer_lock(answer)
    }
}

/// The tree's way to its devices: further connections through the daemon,
/// and the cache lease of the platform.
struct HostChildLink {
    session: Arc<MountHostSession>,
    lease: LeaseFn,
}

impl HostChildLink {
    fn new(session: Arc<MountHostSession>, lease: LeaseFn) -> Self {
        Self { session, lease }
    }
}

impl ChildLink for HostChildLink {
    fn connect(&self, target: &PeerMountTarget, root: &BackendRoot) -> io::Result<ChildConnection> {
        self.session
            .child_backend(target, root)
            .map(|(backend, writable)| ChildConnection { backend, writable })
            .map_err(io::Error::other)
    }

    fn roots(&self, target: &PeerMountTarget) -> io::Result<Vec<ChildRoot>> {
        self.session.child_roots(target).map_err(io::Error::other)
    }

    fn acquire_lease(&self, spool_root: &Path, id: &MountId) -> io::Result<Box<dyn Any + Send>> {
        (self.lease)(spool_root, id)
    }
}

/// Asks the daemon for the device list again and again and hands every new
/// one to the tree. The thread ends with the host; the long poll in flight
/// ends at the latest after its 30 s.
fn start_watcher(feed: Arc<dyn DeviceFeed>, tree: &Arc<DeviceTree>, stop: &Arc<AtomicBool>) {
    let tree: Weak<DeviceTree> = Arc::downgrade(tree);
    let stop = Arc::clone(stop);
    let spawned = std::thread::Builder::new()
        .name("devices-watch".into())
        .spawn(move || {
            let mut since = 0u64;
            while !stop.load(Ordering::Acquire) {
                match feed.watch(since) {
                    Ok((revision, devices)) => {
                        if revision != since {
                            let Some(tree) = tree.upgrade() else {
                                return;
                            };
                            tree.set_devices(devices);
                            since = revision;
                        }
                    }
                    Err(_) => pause(&stop, WATCH_RETRY),
                }
            }
        });
    if let Err(error) = spawned {
        eprintln!("device list watcher not started: {error}");
    }
}

/// Sleeps `total` unless the host ends meanwhile.
fn pause(stop: &AtomicBool, total: Duration) {
    let mut waited = Duration::ZERO;
    while waited < total && !stop.load(Ordering::Acquire) {
        std::thread::sleep(WATCH_STEP);
        waited += WATCH_STEP;
    }
}
