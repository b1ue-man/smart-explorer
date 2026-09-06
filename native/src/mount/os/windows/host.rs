use std::{
    io,
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr::null,
    sync::{Arc, Mutex},
    time::Duration,
};

use windows_sys::Win32::Storage::FileSystem::GetLogicalDrives;

use crate::{
    daemon::{connect_mount_host, MountHostSession},
    mount::{
        prepare_spool_root, DriveLetter, DriveSelection, EntryCondition, MountEngine, MountId,
        MountMode, MountRecovery, MountRuntimeConfig, MountStatus,
    },
};

use super::metadata_refresh::MetadataRefreshWorker;
use super::{
    cache_lease::CacheLease, callback_context::CallbackContext,
    callback_status::CALLBACK_TIMEOUT_MS, callbacks, wide::encode_mount_point, DokanOperations,
    DokanOptions, DokanyCreateError, DokanyFileSystem, DokanyPreflightError, DokanyRuntime,
    DokanyRuntimeInfo, DokanyWaitOutcome, OPTION_CASE_SENSITIVE,
    OPTION_CURRENT_SESSION, OPTION_MOUNT_MANAGER, OPTION_WRITE_PROTECT,
};

const CONTROL_POLL: Duration = Duration::from_millis(250);

#[path = "host_lifecycle.rs"]
mod lifecycle;
use lifecycle::{close_and_finalize, run_until_stopped};

#[cfg(test)]
#[path = "mounted_volume_task_tests.rs"]
mod mount_batching_task;

pub(crate) fn preflight_runtime() -> Result<DokanyRuntimeInfo, DokanyPreflightError> {
    DokanyRuntime::preflight().map(|runtime| runtime.info())
}

pub(crate) fn run_mount_host(id: MountId) -> Result<(), String> {
    let session = Arc::new(connect_mount_host(id)?);
    let config = session.config.clone();
    let spool_root = prepare_spool_root(&session.cache_root).map_err(|_| {
        report_failure(
            &session,
            "the isolated mount cache directory is unavailable",
        )
    })?;
    let cache_lease = CacheLease::acquire(&spool_root, &config.id).map_err(|_| {
        report_failure_with_recovery(
            &session,
            "the mounted-drive cache is already in use or its lease is unsafe",
            MountRecovery::Unknown,
        )
    })?;
    let space = super::cache_space::CacheDiskSpace::new(&spool_root)
        .map_err(|error| report_failure(&session, &format!("cache space probe: {error}")))?;
    let engine = MountEngine::open_host_cache(
        MountRuntimeConfig::new(config.id.clone(), config.mode)
            .with_metadata_policy(config.metadata)
            .with_cache_policy(config.cache)
            .with_runtime_preference(config.runtime_preference),
        Arc::clone(&session.backend),
        &spool_root,
    )
    .map_err(|_| {
        report_failure_with_recovery(
            &session,
            "the mounted-drive cache could not be recovered safely",
            MountRecovery::Unknown,
        )
    })?.with_cache_space_probe(Arc::new(space));
    let local_recovery = inspect_recovery(&engine).map_err(|_| {
        report_failure_with_recovery(
            &session,
            "the mounted-drive cache could not report its recovery state",
            MountRecovery::Unknown,
        )
    })?;
    session.report_status_with_recovery(MountStatus::Mounting, Some(local_recovery))?;
    let selection = match super::runtime_selection::RuntimeSelection::select(
        &spool_root, &config.id, config.runtime_preference,
    ) {
        Ok(selection) => selection,
        Err(error) => {
            let detail = error.to_string();
            let _ = session.report_status(MountStatus::RuntimeUnavailable {
                detail: detail.clone(),
            });
            return Err(detail);
        }
    };
    let runtime = &selection.runtime;
    if let Err(error) = engine.prepare_host_remote() {
        let detail = format!("mounted-drive remote root validation failed: {error}");
        return Err(report_engine_failure(&session, &engine, &detail));
    }
    if let Err(error) = engine.retry_pending_changes() {
        let detail = format!(
            "cached mounted-drive changes could not be retried safely: {error}; keep the cache and use Retry after connectivity is restored"
        );
        return Err(report_engine_failure(&session, &engine, &detail));
    }
    if let Err(error) = engine.preload_metadata() {
        let detail = format!("mounted-drive root metadata preload failed: {error}");
        return Err(report_engine_failure(&session, &engine, &detail));
    }
    let reconciled = inspect_recovery(&engine).map_err(|_| {
        report_failure_with_recovery(
            &session,
            "the mounted-drive cache could not report its reconciled recovery state",
            MountRecovery::Unknown,
        )
    })?;
    session.report_status_with_recovery(MountStatus::Mounting, Some(reconciled))?;
    let candidates =
        drive_candidates(config.drive).map_err(|message| report_failure(&session, message))?;
    let cache_root_wide = absolute_path_wide(&spool_root)
        .map_err(|_| report_failure(&session, "the isolated mount cache path is invalid"))?;
    let initial_drive = candidates
        .first()
        .copied()
        .ok_or_else(|| report_failure(&session, "no Windows drive letter is available"))?;
    let engine = Arc::new(engine);
    let context = Box::new(
        CallbackContext::new(
            Arc::clone(&engine),
            runtime.clone(),
            Arc::clone(&session),
            initial_drive,
            config.mode == MountMode::ReadOnly,
            config.label,
            super::metadata::volume_serial(config.id.as_str()),
            cache_root_wide,
        )
        .map_err(|error| {
            report_engine_failure(
                &session,
                &engine,
                &format!("mounted-drive callback supervisor could not start: {error}"),
            )
        })?,
    );
    let mut storage = CallbackStorage::new(context, config.mode == MountMode::ReadOnly);
    // Arm recovery ownership before Dokany may dispatch its first callback.
    // If creation itself fails, the branch below disarms only after proving
    // that the journal is still clean.
    session.report_status_with_recovery(MountStatus::Mounting, Some(MountRecovery::Unknown))?;
    let filesystem = match start_on_available_drive(runtime, &mut storage, &candidates) {
        Ok(filesystem) => filesystem,
        Err(error) => {
            return Err(report_pre_mount_failure(
                &session,
                &storage.context.engine,
                &format!("Dokany could not mount the drive: {error}"),
            ));
        }
    };
    if let Err(error) = storage.start_metadata_refresh() {
        let detail = format!("mounted-drive metadata refresh could not start: {error}");
        return close_and_finalize(filesystem, &storage, &session, Some(&detail));
    }
    let result = run_until_stopped(filesystem, &storage, &session);
    // Make the host-lifetime ownership boundary explicit: dirty-entry
    // inspection and engine teardown complete before another host may open it.
    drop(storage);
    drop(engine);
    if result.is_ok() {
        selection.complete();
    }
    drop(cache_lease);
    result
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
                &self.context.engine,
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

fn report_failure(session: &MountHostSession, message: &str) -> String {
    let _ = session.report_status(MountStatus::Failed {
        detail: message.to_string(),
    });
    message.to_string()
}

fn report_failure_with_recovery(
    session: &MountHostSession,
    message: &str,
    recovery: MountRecovery,
) -> String {
    let _ = session.report_status_with_recovery(
        MountStatus::Failed {
            detail: message.to_string(),
        },
        Some(recovery),
    );
    message.to_string()
}

fn report_pre_mount_failure(
    session: &MountHostSession,
    engine: &MountEngine,
    message: &str,
) -> String {
    match engine.dirty_entries() {
        Ok(entries) if entries.is_empty() => {
            report_failure_with_recovery(session, message, MountRecovery::Clean)
        }
        Ok(_) => report_failure_with_recovery(session, message, MountRecovery::Required),
        Err(_) => report_failure_with_recovery(session, message, MountRecovery::Unknown),
    }
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

fn report_engine_failure(
    session: &MountHostSession,
    engine: &MountEngine,
    message: &str,
) -> String {
    report_pre_mount_failure(session, engine, message)
}
