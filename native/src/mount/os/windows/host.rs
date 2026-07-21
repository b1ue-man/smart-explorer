use std::{io, os::windows::ffi::OsStrExt, path::Path, ptr::null, sync::Arc, time::Duration};

use windows_sys::Win32::Storage::FileSystem::GetLogicalDrives;

use crate::{
    daemon::{connect_mount_host, MountHostSession},
    mount::{
        prepare_spool_root, DriveLetter, DriveSelection, EntryCondition, MountEngine, MountId,
        MountMode, MountRuntimeConfig, MountStatus,
    },
};

use super::{
    cache_lease::CacheLease, callback_context::CallbackContext,
    callback_status::CALLBACK_TIMEOUT_MS, callbacks, wide::encode_mount_point, DokanOperations,
    DokanOptions, DokanyCreateError, DokanyFileSystem, DokanyPreflightError, DokanyRuntime,
    DokanyRuntimeInfo, DokanyWaitOutcome, OPTION_ALLOW_IPC_BATCHING, OPTION_CASE_SENSITIVE,
    OPTION_CURRENT_SESSION, OPTION_MOUNT_MANAGER, OPTION_WRITE_PROTECT,
};

const CONTROL_POLL: Duration = Duration::from_millis(250);

pub(crate) fn preflight_runtime() -> Result<DokanyRuntimeInfo, DokanyPreflightError> {
    DokanyRuntime::preflight().map(|runtime| runtime.info())
}

pub(crate) fn run_mount_host(id: MountId) -> Result<(), String> {
    let session = Arc::new(connect_mount_host(id)?);
    let runtime = match DokanyRuntime::preflight() {
        Ok(runtime) => runtime,
        Err(error) => {
            let detail = error.to_string();
            let _ = session.report_status(MountStatus::RuntimeUnavailable {
                detail: detail.clone(),
            });
            return Err(detail);
        }
    };
    let config = session.config.clone();
    let spool_root = prepare_spool_root(&session.cache_root).map_err(|_| {
        report_failure(
            &session,
            "the isolated mount cache directory is unavailable",
        )
    })?;
    let cache_lease = CacheLease::acquire(&spool_root, &config.id).map_err(|_| {
        report_failure(
            &session,
            "the mounted-drive cache is already in use or its lease is unsafe",
        )
    })?;
    let engine = MountEngine::open_host(
        MountRuntimeConfig::new(config.id.clone(), config.mode),
        Arc::clone(&session.backend),
        &spool_root,
    )
    .map_err(|_| {
        report_failure(
            &session,
            "the mounted-drive cache could not be recovered safely",
        )
    })?;
    engine.retry_pending_changes().map_err(|_| {
        report_failure(
            &session,
            "cached mounted-drive changes could not be retried safely; keep the cache and use Retry after connectivity is restored",
        )
    })?;
    let recovered = engine.dirty_entries().map_err(|_| {
        report_failure(
            &session,
            "the mounted-drive cache could not report its recovery state",
        )
    })?;
    // Tell the daemon what the secured journal actually contains. A clean
    // pre-mount failure can then be removed, while recovered work remains
    // owned even if no drive letter can be selected.
    session.report_status_with_recovery(MountStatus::Mounting, Some(!recovered.is_empty()))?;
    let candidates =
        drive_candidates(config.drive).map_err(|message| report_failure(&session, message))?;
    let cache_root_wide = absolute_path_wide(&spool_root)
        .map_err(|_| report_failure(&session, "the isolated mount cache path is invalid"))?;
    let initial_drive = candidates
        .first()
        .copied()
        .ok_or_else(|| report_failure(&session, "no Windows drive letter is available"))?;
    let context = Box::new(CallbackContext::new(
        engine,
        runtime.clone(),
        Arc::clone(&session),
        initial_drive,
        config.mode == MountMode::ReadOnly,
        config.label,
        super::metadata::volume_serial(config.id.as_str()),
        cache_root_wide,
    ));
    let mut storage = CallbackStorage::new(context, config.mode == MountMode::ReadOnly);
    // Arm recovery ownership before Dokany may dispatch its first callback.
    // If creation itself fails, the branch below disarms only after proving
    // that the journal is still clean.
    session.report_status_with_recovery(MountStatus::Mounting, Some(true))?;
    let filesystem = match start_on_available_drive(&runtime, &mut storage, &candidates) {
        Ok(filesystem) => filesystem,
        Err(error) => {
            return Err(report_pre_mount_failure(
                &session,
                &storage.context.engine,
                &format!("Dokany could not mount the drive: {error}"),
            ));
        }
    };
    let result = run_until_stopped(filesystem, &storage, &session);
    // Make the host-lifetime ownership boundary explicit: dirty-entry
    // inspection and engine teardown complete before another host may open it.
    drop(storage);
    drop(cache_lease);
    result
}

struct CallbackStorage {
    context: Box<CallbackContext>,
    options: Box<DokanOptions>,
    operations: Box<DokanOperations>,
    mount_point: Vec<u16>,
}

impl CallbackStorage {
    fn new(context: Box<CallbackContext>, read_only: bool) -> Self {
        let mut options = Box::<DokanOptions>::default();
        options.options = OPTION_CURRENT_SESSION
            | OPTION_MOUNT_MANAGER
            | OPTION_ALLOW_IPC_BATCHING
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
            options,
            operations: Box::new(callbacks::operations()),
            mount_point: Vec::new(),
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

fn run_until_stopped(
    filesystem: DokanyFileSystem,
    storage: &CallbackStorage,
    session: &MountHostSession,
) -> Result<(), String> {
    let drive = match storage.context.selected_drive() {
        Ok(drive) => drive,
        Err(_) => {
            return close_and_finalize(
                filesystem,
                storage,
                session,
                Some("the mounted drive lost its letter state"),
            );
        }
    };
    let initial_status = match storage.context.engine.dirty_entries() {
        Ok(entries) if entries.is_empty() => MountStatus::Mounted { drive },
        Ok(entries) => {
            let (path, condition) = &entries[0];
            let detail = match condition {
                EntryCondition::Conflict(conflict) => conflict.detail.clone(),
                EntryCondition::Dirty => {
                    "a cached change still requires safe remote recovery".into()
                }
                EntryCondition::Clean => "mounted-drive recovery is incomplete".into(),
            };
            MountStatus::Conflict {
                drive,
                path: path.clone(),
                detail,
            }
        }
        Err(_) => {
            return close_and_finalize(
                filesystem,
                storage,
                session,
                Some("the mounted drive could not inspect its recovery state"),
            );
        }
    };
    if session.report_status(initial_status).is_err() {
        return close_and_finalize(
            filesystem,
            storage,
            session,
            Some("the mounted drive could not report its running state"),
        );
    }
    loop {
        if session.wait_for_stop_timeout(CONTROL_POLL) {
            return close_and_finalize(filesystem, storage, session, None);
        }
        if storage.context.stop_requested() {
            return close_and_finalize(
                filesystem,
                storage,
                session,
                Some("the mounted drive stopped after a callback or control failure"),
            );
        }
        match filesystem.wait(0) {
            DokanyWaitOutcome::Timeout if filesystem.is_running() => {}
            DokanyWaitOutcome::Closed | DokanyWaitOutcome::Timeout => {
                return close_and_finalize(filesystem, storage, session, None);
            }
            DokanyWaitOutcome::Failed { .. } | DokanyWaitOutcome::Unexpected(_) => {
                return close_and_finalize(
                    filesystem,
                    storage,
                    session,
                    Some("Dokany stopped the mounted drive unexpectedly"),
                );
            }
        }
    }
}

fn close_and_finalize(
    filesystem: DokanyFileSystem,
    storage: &CallbackStorage,
    session: &MountHostSession,
    prior_failure: Option<&'static str>,
) -> Result<(), String> {
    // DokanCloseHandle is the callback lifetime boundary. Only inspect the
    // engine once Dokany can no longer mutate its retryable journal state.
    filesystem.close();
    match storage.context.engine.dirty_entries() {
        Ok(entries) if !entries.is_empty() => {
            let detail = recovery_detail(&entries);
            Err(report_failure_with_recovery(session, &detail, true))
        }
        Err(_) => Err(report_failure(
            session,
            "the mounted drive closed, but its recovery journal could not be inspected; keep the mount and use Retry",
        )),
        Ok(_) => match prior_failure {
            Some(detail) => Err(report_failure_with_recovery(session, detail, false)),
            None => session
                .report_status_with_recovery(MountStatus::Unmounted, Some(false))
                .map_err(|_| "the mounted drive closed, but its final status could not be recorded".to_string()),
        },
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
    recovery_required: bool,
) -> String {
    let _ = session.report_status_with_recovery(
        MountStatus::Failed {
            detail: message.to_string(),
        },
        Some(recovery_required),
    );
    message.to_string()
}

fn report_pre_mount_failure(
    session: &MountHostSession,
    engine: &MountEngine,
    message: &str,
) -> String {
    match engine.dirty_entries() {
        Ok(entries) => report_failure_with_recovery(session, message, !entries.is_empty()),
        Err(_) => report_failure(session, message),
    }
}
