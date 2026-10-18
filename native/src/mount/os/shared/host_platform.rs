//! The platform half of the shared drive-host flow (`host_session.rs`,
//! DRV-M2). A platform owns everything its filesystem frontend needs: the
//! cache lease, the free-space probe, the runtime (Dokany DLL, FUSE device),
//! the mounted volume and its change delivery. `host_session.rs` calls these
//! steps in one fixed order for every platform: the order of the Windows
//! host of v0.5.182.
use std::any::Any;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::daemon::MountHostConfig;
use crate::mount::{CacheSpaceProbe, DriveLetter, MountEngine, MountId, PathDialect};

/// Exclusive ownership of one drive cache; released only after the engine
/// and every frontend reference to it have ended.
pub(crate) type HostLease = Box<dyn Any + Send>;

/// Where the mounted volume is reachable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum VolumeLocation {
    Drive(DriveLetter),
    /// A folder (Linux FUSE, DRV-M4); reported as `MountStatus::MountedAt`.
    Directory(PathBuf),
}

/// Failure of a platform step before the volume exists, with the status
/// report it needs.
#[derive(Debug)]
pub(crate) enum HostStepError {
    /// `Failed` without a recovery value (the journal is untouched).
    Plain(String),
    /// `Failed` with the recovery value of the engine's journal.
    Engine(String),
}

/// State of a running volume between two control polls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VolumePoll {
    Running,
    /// The volume ended without an unmount request; the text is the failure
    /// detail the host reports.
    Ended(&'static str),
    /// The volume was unmounted outside the host (a file manager's eject,
    /// `fusermount3 -u`); the host ends like on an unmount request.
    Unmounted,
}

pub(crate) trait HostPlatform {
    /// Name rules of this frontend.
    fn path_dialect(&self) -> PathDialect;
    fn acquire_lease(&mut self, spool_root: &Path, id: &MountId) -> io::Result<HostLease>;
    fn space_probe(&mut self, spool_root: &Path) -> io::Result<Arc<dyn CacheSpaceProbe>>;
    /// Loads the filesystem runtime; the error text becomes
    /// `RuntimeUnavailable`.
    fn select_runtime(&mut self, spool_root: &Path, config: &MountHostConfig)
        -> Result<(), String>;
    /// Builds the frontend state over `engine` without exposing it yet.
    fn prepare_volume(
        &mut self,
        engine: &Arc<MountEngine>,
        config: &MountHostConfig,
        spool_root: &Path,
    ) -> Result<(), HostStepError>;
    /// Makes the volume visible; from here on callbacks may arrive.
    fn start_volume(&mut self) -> Result<(), String>;
    /// Starts the background metadata refresh of the volume.
    fn start_background(&mut self) -> io::Result<()>;
    fn location(&self) -> io::Result<VolumeLocation>;
    /// A callback or its supervision asked the host to stop.
    fn stop_requested(&self) -> bool;
    fn poll_volume(&mut self) -> VolumePoll;
    /// Delivers drained remote changes to the frontend.
    fn deliver_changes(&mut self) -> io::Result<()>;
    /// Stops background work and closes the volume like an unmount without
    /// force; what the frontend could not publish stays in the journal.
    /// `false` when the shutdown watchdog failed.
    fn shutdown(&mut self) -> bool;
    /// Drops the frontend state and with it its engine references.
    fn release_volume(&mut self);
    /// Ends the runtime after the volume; `completed` only after a
    /// controlled teardown with a clean journal.
    fn finish_runtime(&mut self, completed: bool);
    /// Why the volume ended on its own, when the frontend knows more than
    /// the `VolumePoll::Ended` text (Linux: the kernel request timeout). The
    /// final failure report leads with it.
    fn failure_cause(&self) -> Option<String> {
        None
    }
}
