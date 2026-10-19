//! The Windows drive host (`--mount-host`): Dokany as the platform of the
//! shared host flow (`os/shared/host_session.rs`, DRV-M2). The order of the
//! steps, the status reports and the teardown live there; this file owns the
//! Dokany runtime, the callback state, the drive letter and the volume.
use std::{
    io,
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr::null,
    sync::{Arc, Mutex},
};

use windows_sys::Win32::Storage::FileSystem::GetLogicalDrives;

use crate::{
    daemon::{connect_mount_host, MountHostConfig},
    mount::{
        CacheSpaceProbe, DriveLetter, DriveSelection, MountEngine, MountFilesystem, MountId,
        MountMode, PathDialect,
    },
};

use super::super::shared::host_platform::{
    HostLease, HostPlatform, HostStepError, LeaseFn, VolumeLocation, VolumePoll,
};
use super::super::shared::host_session::{self, HostExit, HostInputs, UpdateStopCheck};
use super::callback_reporter::CallbackReporter;
use super::metadata_refresh::MetadataRefreshWorker;
use super::notifications::HostNotifications;
use super::runtime_selection::RuntimeSelection;
use super::shutdown_watchdog::ShutdownWatchdog;
use super::{
    cache_lease::CacheLease, callback_context::CallbackContext,
    callback_status::CALLBACK_TIMEOUT_MS, callbacks, wide::encode_mount_point, DokanOperations,
    DokanOptions, DokanyCreateError, DokanyFileSystem, DokanyPreflightError, DokanyRuntime,
    DokanyRuntimeInfo, DokanyWaitOutcome, OPTION_CASE_SENSITIVE, OPTION_CURRENT_SESSION,
    OPTION_MOUNT_MANAGER, OPTION_WRITE_PROTECT,
};

// Names the Dokany volume harnesses take from this module through
// `use super::*` and production code here no longer uses.
#[cfg(test)]
use crate::mount::{prepare_spool_root, MountRuntimeConfig, MountStatus};
#[cfg(test)]
use std::time::Duration;

#[cfg(test)]
#[path = "mounted_volume_task_tests.rs"]
mod mount_batching_task;

#[cfg(test)]
#[path = "optimization_volume_tests.rs"]
mod optimization_task;

#[cfg(test)]
#[path = "host_session_windows_tests.rs"]
mod host_session_windows;

// DRV-M6: the device tree over Dokany.
#[cfg(test)]
#[path = "devices_drive_task_tests.rs"]
mod devices_drive_task;

// DRV-M7, VLT-M12: vault drives over Dokany.
#[cfg(test)]
#[path = "vault_drive_task_tests.rs"]
mod vault_drive_task;

pub(crate) fn preflight_runtime() -> Result<DokanyRuntimeInfo, DokanyPreflightError> {
    DokanyRuntime::preflight().map(|runtime| runtime.info())
}

pub(crate) fn run_mount_host(id: MountId) -> Result<HostExit, String> {
    // V8: the update-stop baseline is the marker present when the process
    // starts; a marker an earlier update left behind never stops this host.
    let update_stop = UpdateStopCheck::arm();
    let session = Arc::new(connect_mount_host(id)?);
    let inputs = HostInputs::from_session(&session);
    let mut platform = DokanyHost::new(CallbackReporter::from(Arc::clone(&session)));
    // DRV-M6: the device tree has its own host flow over the same platform.
    if session.config.device_tree {
        return host_session::devices::run(inputs, &session, &mut platform, &update_stop);
    }
    host_session::run(inputs, &*session, &mut platform, &update_stop)
}

/// Dokany as platform of the shared host flow.
pub(super) struct DokanyHost {
    // Field order is drop order: the volume closes before the callback state
    // it points into, and both end before the runtime DLL.
    filesystem: Option<DokanyFileSystem>,
    storage: Option<CallbackStorage>,
    notifications: HostNotifications,
    selection: Option<RuntimeSelection>,
    reporter: Option<CallbackReporter>,
    candidates: Vec<DriveLetter>,
}

impl DokanyHost {
    pub(super) fn new(reporter: CallbackReporter) -> Self {
        Self {
            filesystem: None,
            storage: None,
            notifications: HostNotifications::default(),
            selection: None,
            reporter: Some(reporter),
            candidates: Vec::new(),
        }
    }

    fn storage(&self) -> io::Result<&CallbackStorage> {
        self.storage
            .as_ref()
            .ok_or_else(|| io::Error::other("the mounted volume was not prepared"))
    }

    /// The callback state of a volume; `build` makes the context over the
    /// engine of a drive (`prepare_volume`) or over the device tree
    /// (`prepare_filesystem`).
    fn prepare(
        &mut self,
        config: &MountHostConfig,
        spool_root: &Path,
        build: impl FnOnce(
            DokanyRuntime,
            CallbackReporter,
            DriveLetter,
            bool,
            String,
            u32,
            Vec<u16>,
        ) -> io::Result<CallbackContext>,
    ) -> Result<(), HostStepError> {
        let runtime = self
            .selection
            .as_ref()
            .map(|selection| selection.runtime.clone())
            .ok_or_else(|| HostStepError::Plain("the Dokany runtime was not selected".into()))?;
        let candidates = drive_candidates(config.drive)
            .map_err(|message| HostStepError::Plain(message.into()))?;
        let cache_root_wide = absolute_path_wide(spool_root)
            .map_err(|_| HostStepError::Plain("the isolated mount cache path is invalid".into()))?;
        let initial_drive = candidates
            .first()
            .copied()
            .ok_or_else(|| HostStepError::Plain("no Windows drive letter is available".into()))?;
        let reporter = self.reporter.take().ok_or_else(|| {
            HostStepError::Plain("the mounted-drive status channel is unavailable".into())
        })?;
        let read_only = config.mode == MountMode::ReadOnly;
        let context = build(
            runtime,
            reporter,
            initial_drive,
            read_only,
            config.label.clone(),
            super::metadata::volume_serial(config.id.as_str()),
            cache_root_wide,
        )
        .map_err(|error| {
            HostStepError::Engine(format!(
                "mounted-drive callback supervisor could not start: {error}"
            ))
        })?;
        self.storage = Some(CallbackStorage::new(Box::new(context), read_only));
        self.candidates = candidates;
        Ok(())
    }
}

impl HostPlatform for DokanyHost {
    fn path_dialect(&self) -> PathDialect {
        PathDialect::Windows
    }

    fn acquire_lease(&mut self, spool_root: &Path, id: &MountId) -> io::Result<HostLease> {
        CacheLease::acquire(spool_root, id).map(|lease| Box::new(lease) as HostLease)
    }

    fn space_probe(&mut self, spool_root: &Path) -> io::Result<Arc<dyn CacheSpaceProbe>> {
        super::cache_space::CacheDiskSpace::new(spool_root)
            .map(|space| Arc::new(space) as Arc<dyn CacheSpaceProbe>)
    }

    fn select_runtime(
        &mut self,
        spool_root: &Path,
        config: &MountHostConfig,
    ) -> Result<(), String> {
        let selection = RuntimeSelection::select(spool_root, &config.id, config.runtime_preference)
            .map_err(|error| error.to_string())?;
        self.selection = Some(selection);
        Ok(())
    }

    fn lease_fn(&self) -> Option<LeaseFn> {
        Some(Arc::new(|spool_root: &Path, id: &MountId| {
            CacheLease::acquire(spool_root, id).map(|lease| Box::new(lease) as HostLease)
        }))
    }

    fn prepare_volume(
        &mut self,
        engine: &Arc<MountEngine>,
        config: &MountHostConfig,
        spool_root: &Path,
    ) -> Result<(), HostStepError> {
        let engine = Arc::clone(engine);
        self.prepare(
            config,
            spool_root,
            move |runtime, reporter, drive, read_only, label, serial, cache| {
                CallbackContext::new(
                    engine, runtime, reporter, drive, read_only, label, serial, cache,
                )
            },
        )
    }

    fn prepare_filesystem(
        &mut self,
        filesystem: Arc<dyn MountFilesystem>,
        config: &MountHostConfig,
        spool_root: &Path,
    ) -> Result<(), HostStepError> {
        self.prepare(
            config,
            spool_root,
            move |runtime, reporter, drive, read_only, label, serial, cache| {
                CallbackContext::for_filesystem(
                    filesystem, runtime, reporter, drive, read_only, label, serial, cache,
                )
            },
        )
    }

    fn start_volume(&mut self) -> Result<(), String> {
        let (Some(selection), Some(storage)) = (self.selection.as_ref(), self.storage.as_mut())
        else {
            return Err("Dokany could not mount the drive: the volume was not prepared".into());
        };
        let filesystem = start_on_available_drive(&selection.runtime, storage, &self.candidates)
            .map_err(|error| format!("Dokany could not mount the drive: {error}"))?;
        self.filesystem = Some(filesystem);
        Ok(())
    }

    fn start_background(&mut self) -> io::Result<()> {
        self.storage()?.start_metadata_refresh()
    }

    fn location(&self) -> io::Result<VolumeLocation> {
        self.storage()?
            .context
            .selected_drive()
            .map(VolumeLocation::Drive)
    }

    fn stop_requested(&self) -> bool {
        self.storage
            .as_ref()
            .is_some_and(|storage| storage.context.stop_requested())
    }

    fn poll_volume(&mut self) -> VolumePoll {
        let Some(filesystem) = self.filesystem.as_ref() else {
            return VolumePoll::Ended("the mounted drive closed without a requested unmount");
        };
        match filesystem.wait(0) {
            DokanyWaitOutcome::Timeout if filesystem.is_running() => VolumePoll::Running,
            DokanyWaitOutcome::Closed | DokanyWaitOutcome::Timeout => {
                VolumePoll::Ended("the mounted drive closed without a requested unmount")
            }
            DokanyWaitOutcome::Failed { .. } | DokanyWaitOutcome::Unexpected(_) => {
                VolumePoll::Ended("Dokany stopped the mounted drive unexpectedly")
            }
        }
    }

    fn deliver_changes(&mut self) -> io::Result<()> {
        let (Some(storage), Some(filesystem)) = (self.storage.as_ref(), self.filesystem.as_ref())
        else {
            return Ok(());
        };
        let drive = storage.context.selected_drive()?;
        self.notifications
            .deliver_from(&*storage.context.filesystem, filesystem, drive)
    }

    fn shutdown(&mut self) -> bool {
        if let Some(storage) = self.storage.as_ref() {
            storage.request_metadata_refresh_stop();
        }
        let watchdog = ShutdownWatchdog::start("dokan-close");
        // DokanCloseHandle is the callback lifetime boundary. Only inspect the
        // engine once Dokany can no longer mutate its retryable journal state.
        if let Some(filesystem) = self.filesystem.take() {
            filesystem.close();
        }
        // Closing Dokany first prevents a slow background target from keeping
        // the drive visible. The worker observes cancellation between remote
        // targets; join still owns its engine before recovery inspection.
        if let Some(watchdog) = watchdog.as_ref() {
            watchdog.set_phase("metadata-refresh");
        }
        if let Some(storage) = self.storage.as_ref() {
            storage.join_metadata_refresh();
        }
        watchdog.is_none_or(ShutdownWatchdog::finish)
    }

    fn release_volume(&mut self) {
        // The volume closes before the callback state it points into.
        self.filesystem = None;
        self.storage = None;
    }

    fn finish_runtime(&mut self, completed: bool) {
        if let Some(selection) = self.selection.take() {
            if completed {
                selection.complete();
            }
        }
    }
}

struct CallbackStorage {
    context: Box<CallbackContext>,
    metadata_refresh: Mutex<Option<MetadataRefreshWorker>>,
    options: Box<DokanOptions>,
    operations: Box<DokanOperations>,
    mount_point: Vec<u16>,
}

impl CallbackStorage {
    fn new(context: Box<CallbackContext>, read_only: bool) -> Self {
        let mut options = Box::<DokanOptions>::default();
        options.options = OPTION_CURRENT_SESSION
            | OPTION_MOUNT_MANAGER
            | if context.case_sensitive_paths {
                OPTION_CASE_SENSITIVE
            } else {
                0
            }
            | if read_only { OPTION_WRITE_PROTECT } else { 0 };
        options.timeout = CALLBACK_TIMEOUT_MS;
        options.allocation_unit_size = 4096;
        options.sector_size = 4096;
        options.unc_name = null();
        options.global_context = (&*context as *const CallbackContext) as usize as u64;
        Self {
            context,
            metadata_refresh: Mutex::new(None),
            options,
            operations: Box::new(callbacks::operations()),
            mount_point: Vec::new(),
        }
    }

    fn start_metadata_refresh(&self) -> io::Result<()> {
        let mut worker = self
            .metadata_refresh
            .lock()
            .map_err(|_| io::Error::other("metadata refresh worker state is unavailable"))?;
        if worker.is_none() {
            *worker = Some(MetadataRefreshWorker::start(Arc::clone(
                &self.context.filesystem,
            ))?);
        }
        Ok(())
    }

    fn request_metadata_refresh_stop(&self) {
        if let Ok(worker) = self.metadata_refresh.lock() {
            if let Some(worker) = worker.as_ref() {
                worker.request_stop();
            }
        }
    }

    fn join_metadata_refresh(&self) {
        if let Ok(mut worker) = self.metadata_refresh.lock() {
            if let Some(worker) = worker.take() {
                worker.join();
            }
        }
    }

    fn select_drive(&mut self, drive: DriveLetter) -> io::Result<()> {
        self.context.set_selected_drive(drive)?;
        self.mount_point = encode_mount_point(drive.get());
        self.options.mount_point = self.mount_point.as_ptr();
        Ok(())
    }
}

fn start_on_available_drive(
    runtime: &DokanyRuntime,
    storage: &mut CallbackStorage,
    candidates: &[DriveLetter],
) -> Result<DokanyFileSystem, DokanyCreateError> {
    let mut last_busy = None;
    for drive in candidates {
        storage
            .select_drive(*drive)
            .map_err(|_| DokanyCreateError::InvalidDriveLetter)?;
        // All callback-owned values are heap-backed and remain stable until
        // DokanCloseHandle returns. A failed creation owns no filesystem.
        let created = unsafe {
            runtime.create_file_system_raw(&mut *storage.options, &mut *storage.operations)
        };
        match created {
            Ok(filesystem) => return Ok(filesystem),
            Err(error @ DokanyCreateError::InvalidDriveLetter) => last_busy = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(last_busy.unwrap_or(DokanyCreateError::InvalidDriveLetter))
}

fn drive_candidates(selection: DriveSelection) -> Result<Vec<DriveLetter>, &'static str> {
    if let DriveSelection::Letter(letter) = selection {
        let occupied = unsafe { GetLogicalDrives() } & (1u32 << (letter.get() as u8 - b'A')) != 0;
        return if occupied {
            Err("the selected Windows drive letter is already in use")
        } else {
            Ok(vec![letter])
        };
    }
    let occupied = unsafe { GetLogicalDrives() };
    Ok((b'D'..=b'Z')
        .rev()
        .filter(|letter| occupied & (1u32 << (*letter - b'A')) == 0)
        .filter_map(|letter| DriveLetter::parse(letter as char).ok())
        .collect())
}

fn absolute_path_wide(path: &Path) -> io::Result<Vec<u16>> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "mount cache path is not absolute",
        ));
    }
    let mut encoded = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if encoded.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "mount cache path contains NUL",
        ));
    }
    encoded.push(0);
    Ok(encoded)
}

#[cfg(test)]
#[path = "windows_remote_volume_task_tests.rs"]
mod windows_remote_task;
