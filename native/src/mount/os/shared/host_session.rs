//! The shared drive-host flow (DRV-M2): one order of steps for every
//! filesystem frontend, taken unchanged from the Windows host of v0.5.182
//! (`host.rs`, `host_lifecycle.rs`): cache lease, space probe, engine, local
//! recovery report, runtime, remote root check, retry of pending changes,
//! root preload, reconciled report, frontend state, recovery ownership,
//! volume, background refresh, running status, control loop, teardown.
//!
//! Update stop (contract V8): the control loop reads `daemon.stop` once per
//! second against the baseline taken when the host process started. A new
//! request closes the volume exactly like an unmount without `--force` —
//! what the frontend can publish is published, everything else stays in
//! the journal for Retry — and the host ends with `EXIT_UPDATE_STOP` (75),
//! so no supervisor restarts it and the updater can replace the image.
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::daemon::{
    MountHostConfig, MountHostSession, UpdateStopBaseline, EXIT_UPDATE_STOP, UPDATE_STOP_POLL,
};
use crate::mount::{
    prepare_spool_root, EntryCondition, MountEngine, MountRecovery, MountRuntimeConfig,
    MountStatus, MountStorage,
};
use crate::vfs::BackendHandle;

use super::host_platform::{HostPlatform, HostStepError, VolumeLocation, VolumePoll};

/// One control wait between two loop turns (value of v0.5.182).
pub(crate) const CONTROL_POLL: Duration = Duration::from_millis(250);

/// The daemon side of a drive host.
pub(crate) trait HostControl {
    fn report(&self, status: MountStatus, recovery: Option<MountRecovery>) -> Result<(), String>;
    /// `true` for an unmount request and for a lost daemon connection.
    fn wait_for_stop(&self, timeout: Duration) -> bool;
}

impl HostControl for MountHostSession {
    fn report(&self, status: MountStatus, recovery: Option<MountRecovery>) -> Result<(), String> {
        self.report_status_with_recovery(status, recovery)
    }

    fn wait_for_stop(&self, timeout: Duration) -> bool {
        self.wait_for_stop_timeout(timeout)
    }
}

/// Why a host ended without an error of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostExit {
    /// Unmount request or lost daemon connection.
    Stopped,
    /// A new update-stop request (V8).
    UpdateStop,
}

impl HostExit {
    pub(crate) fn exit_code(self) -> i32 {
        match self {
            Self::Stopped => 0,
            Self::UpdateStop => EXIT_UPDATE_STOP,
        }
    }
}

/// The update-stop baseline of this host process and how often it is read.
pub(crate) struct UpdateStopCheck {
    baseline: UpdateStopBaseline,
    interval: Duration,
}

impl UpdateStopCheck {
    /// Baseline now; call it before anything else in the host process, so a
    /// marker left by an earlier update is ignored.
    pub(crate) fn arm() -> Self {
        Self::new(UpdateStopBaseline::arm(), UPDATE_STOP_POLL)
    }

    pub(crate) fn new(baseline: UpdateStopBaseline, interval: Duration) -> Self {
        Self { baseline, interval }
    }
}

/// What the daemon handed to the host.
pub(crate) struct HostInputs {
    pub(crate) config: MountHostConfig,
    pub(crate) backend: BackendHandle,
    pub(crate) cache_root: PathBuf,
}

impl HostInputs {
    pub(crate) fn from_session(session: &MountHostSession) -> Self {
        Self {
            config: session.config.clone(),
            backend: Arc::clone(&session.backend),
            cache_root: session.cache_root.clone(),
        }
    }
}

/// Result of the leased part of a run: the teardown and why the host ended.
struct Ended {
    teardown: Result<(), String>,
    exit: HostExit,
}

impl Ended {
    fn failed(detail: String) -> Self {
        Self {
            teardown: Err(detail),
            exit: HostExit::Stopped,
        }
    }
}

/// Runs one drive host to its end. An update stop ends with
/// `Ok(HostExit::UpdateStop)` even when changes stay in the journal: its
/// final status (`Failed` with `MountRecovery::Required`) is reported.
pub(crate) fn run(
    inputs: HostInputs,
    control: &dyn HostControl,
    platform: &mut dyn HostPlatform,
    update_stop: &UpdateStopCheck,
) -> Result<HostExit, String> {
    let HostInputs {
        config,
        backend,
        cache_root,
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
        backend,
        &spool_root,
        control,
        platform,
        update_stop,
    );
    // The frontend state holds the last engine references; it and the runtime
    // end before another host may take the cache.
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
    backend: BackendHandle,
    spool_root: &Path,
    control: &dyn HostControl,
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
    let runtime = MountRuntimeConfig::new(config.id.clone(), config.mode)
        .with_metadata_policy(config.metadata)
        .with_cache_policy(config.cache)
        .with_runtime_preference(config.runtime_preference)
        .with_path_dialect(dialect);
    // A vault drive (DRV-M7) passes its sealed storage here.
    let engine = match MountEngine::open_host_cache_with(
        runtime,
        backend,
        spool_root,
        MountStorage::plain(),
    ) {
        Ok(engine) => engine.with_cache_space_probe(space),
        Err(_) => {
            return Ended::failed(report_failure_with_recovery(
                control,
                "the mounted-drive cache could not be recovered safely",
                MountRecovery::Unknown,
            ))
        }
    };
    let Ok(local_recovery) = inspect_recovery(&engine) else {
        return Ended::failed(report_failure_with_recovery(
            control,
            "the mounted-drive cache could not report its recovery state",
            MountRecovery::Unknown,
        ));
    };
    if let Err(error) = control.report(MountStatus::Mounting, Some(local_recovery)) {
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
    if let Err(error) = engine.prepare_host_remote() {
        let detail = format!("mounted-drive remote root validation failed: {error}");
        return Ended::failed(report_engine_failure(control, &engine, &detail));
    }
    if let Err(error) = engine.retry_pending_changes() {
        let detail = format!(
            "cached mounted-drive changes could not be retried safely: {error}; keep the cache and use Retry after connectivity is restored"
        );
        return Ended::failed(report_engine_failure(control, &engine, &detail));
    }
    if let Err(error) = engine.preload_metadata() {
        let detail = format!("mounted-drive root metadata preload failed: {error}");
        return Ended::failed(report_engine_failure(control, &engine, &detail));
    }
    let Ok(reconciled) = inspect_recovery(&engine) else {
        return Ended::failed(report_failure_with_recovery(
            control,
            "the mounted-drive cache could not report its reconciled recovery state",
            MountRecovery::Unknown,
        ));
    };
    if let Err(error) = control.report(MountStatus::Mounting, Some(reconciled)) {
        return Ended::failed(error);
    }
    let engine = Arc::new(engine);
    if let Err(error) = platform.prepare_volume(&engine, config, spool_root) {
        return Ended::failed(match error {
            HostStepError::Plain(detail) => report_failure(control, &detail),
            HostStepError::Engine(detail) => report_engine_failure(control, &engine, &detail),
        });
    }
    // Arm recovery ownership before the frontend may dispatch its first
    // callback. If the volume cannot start, the failure report disarms only
    // after proving that the journal is still clean.
    if let Err(error) = control.report(MountStatus::Mounting, Some(MountRecovery::Unknown)) {
        return Ended::failed(error);
    }
    if let Err(detail) = platform.start_volume() {
        return Ended::failed(report_engine_failure(control, &engine, &detail));
    }
    if let Err(error) = platform.start_background() {
        let detail = format!("mounted-drive metadata refresh could not start: {error}");
        return finish(platform, control, &engine, Some(&detail), HostExit::Stopped);
    }
    run_until_stopped(platform, control, &engine, update_stop)
}

fn run_until_stopped(
    platform: &mut dyn HostPlatform,
    control: &dyn HostControl,
    engine: &MountEngine,
    update_stop: &UpdateStopCheck,
) -> Ended {
    let Ok(location) = platform.location() else {
        let detail = "the mounted drive lost its letter state";
        return finish(platform, control, engine, Some(detail), HostExit::Stopped);
    };
    let initial_status = match engine.dirty_entries() {
        Ok(entries) if entries.is_empty() => mounted_status(location),
        Ok(entries) => conflict_status(location, &entries[0]),
        Err(_) => {
            let detail = "the mounted drive could not inspect its recovery state";
            return finish(platform, control, engine, Some(detail), HostExit::Stopped);
        }
    };
    if control.report(initial_status, None).is_err() {
        let detail = "the mounted drive could not report its running state";
        return finish(platform, control, engine, Some(detail), HostExit::Stopped);
    }
    let mut notification_error_reported = false;
    let mut update_checked = Instant::now();
    loop {
        if control.wait_for_stop(CONTROL_POLL) {
            return finish(platform, control, engine, None, HostExit::Stopped);
        }
        if update_checked.elapsed() >= update_stop.interval {
            update_checked = Instant::now();
            if update_stop.baseline.requested() {
                return finish(platform, control, engine, None, HostExit::UpdateStop);
            }
        }
        if platform.stop_requested() {
            let detail = "the mounted drive stopped after a callback or control failure";
            return finish(platform, control, engine, Some(detail), HostExit::Stopped);
        }
        match platform.poll_volume() {
            VolumePoll::Running => {}
            VolumePoll::Ended(detail) => {
                return finish(platform, control, engine, Some(detail), HostExit::Stopped)
            }
            // Ejected outside the host (DRV-M4): like an unmount request.
            VolumePoll::Unmounted => {
                return finish(platform, control, engine, None, HostExit::Stopped)
            }
        }
        match platform.deliver_changes() {
            Ok(()) => notification_error_reported = false,
            Err(error) if !notification_error_reported => {
                let _ = writeln!(
                    io::stderr().lock(),
                    "mount change delivery deferred: {error}"
                );
                notification_error_reported = true;
            }
            Err(_) => {}
        }
    }
}

fn finish(
    platform: &mut dyn HostPlatform,
    control: &dyn HostControl,
    engine: &MountEngine,
    prior_failure: Option<&str>,
    exit: HostExit,
) -> Ended {
    Ended {
        teardown: close_and_finalize(platform, control, engine, prior_failure),
        exit,
    }
}

/// Closes the volume, then reports the final state from the journal: open
/// changes keep the cache for Retry; a clean journal reports `Unmounted`.
fn close_and_finalize(
    platform: &mut dyn HostPlatform,
    control: &dyn HostControl,
    engine: &MountEngine,
    prior_failure: Option<&str>,
) -> Result<(), String> {
    let watchdog_ok = platform.shutdown();
    let prior_failure = prior_failure.or((!watchdog_ok)
        .then_some("the mounted drive shutdown watchdog stopped after an internal failure"));
    // Only a frontend that knows the cause better than its poll text sets
    // one (Linux kernel request timeout); every other report is unchanged.
    let cause = platform.failure_cause();
    let prior_failure = cause.as_deref().or(prior_failure);
    match engine.dirty_entries() {
        Ok(entries) if !entries.is_empty() => Err(report_failure_with_recovery(
            control,
            &match &cause {
                Some(cause) => format!("{cause}; {}", recovery_detail(&entries)),
                None => recovery_detail(&entries),
            },
            MountRecovery::Required,
        )),
        Err(_) => Err(report_failure_with_recovery(
            control,
            "the mounted drive closed, but its recovery journal could not be inspected; keep the mount and use Retry",
            MountRecovery::Unknown,
        )),
        Ok(_) => match prior_failure {
            Some(detail) => Err(report_failure_with_recovery(
                control,
                detail,
                MountRecovery::Clean,
            )),
            None => control
                .report(MountStatus::Unmounted, Some(MountRecovery::Clean))
                .map_err(|_| {
                    "the mounted drive closed, but its final status could not be recorded"
                        .to_string()
                }),
        },
    }
}

fn mounted_status(location: VolumeLocation) -> MountStatus {
    match location {
        VolumeLocation::Drive(drive) => MountStatus::Mounted { drive },
        VolumeLocation::Directory(path) => MountStatus::MountedAt { path },
    }
}

fn conflict_status(location: VolumeLocation, first: &(String, EntryCondition)) -> MountStatus {
    let (path, condition) = first;
    let detail = match condition {
        EntryCondition::Conflict(conflict) => conflict.detail.clone(),
        EntryCondition::Dirty => "a cached change still requires safe remote recovery".into(),
        EntryCondition::Clean => "mounted-drive recovery is incomplete".into(),
    };
    match location {
        VolumeLocation::Drive(drive) => MountStatus::Conflict {
            drive,
            path: path.clone(),
            detail,
        },
        // `Conflict` names a drive letter; a folder drive reports where it
        // is, and the journal keeps the conflict for the unmount report.
        VolumeLocation::Directory(path) => MountStatus::MountedAt { path },
    }
}

fn recovery_detail(entries: &[(String, EntryCondition)]) -> String {
    let dirty = entries
        .iter()
        .filter(|(_, condition)| matches!(condition, EntryCondition::Dirty))
        .count();
    let conflicts = entries
        .iter()
        .filter(|(_, condition)| matches!(condition, EntryCondition::Conflict(_)))
        .count();
    format!(
        "{} cached change(s) remain after unmount ({} dirty, {} conflict); keep the mount and use Retry",
        entries.len(),
        dirty,
        conflicts
    )
}

fn report_failure(control: &dyn HostControl, message: &str) -> String {
    let _ = control.report(
        MountStatus::Failed {
            detail: message.to_string(),
        },
        None,
    );
    message.to_string()
}

fn report_failure_with_recovery(
    control: &dyn HostControl,
    message: &str,
    recovery: MountRecovery,
) -> String {
    let _ = control.report(
        MountStatus::Failed {
            detail: message.to_string(),
        },
        Some(recovery),
    );
    message.to_string()
}

/// A failure before the volume exists: the recovery value says whether the
/// journal still owns changes.
fn report_engine_failure(control: &dyn HostControl, engine: &MountEngine, message: &str) -> String {
    let recovery = match engine.dirty_entries() {
        Ok(entries) if entries.is_empty() => MountRecovery::Clean,
        Ok(_) => MountRecovery::Required,
        Err(_) => MountRecovery::Unknown,
    };
    report_failure_with_recovery(control, message, recovery)
}

fn inspect_recovery(engine: &MountEngine) -> io::Result<MountRecovery> {
    engine.dirty_entries().map(|entries| {
        if entries.is_empty() {
            MountRecovery::Clean
        } else {
            MountRecovery::Required
        }
    })
}

#[cfg(test)]
#[path = "host_session_tests.rs"]
mod tests;
