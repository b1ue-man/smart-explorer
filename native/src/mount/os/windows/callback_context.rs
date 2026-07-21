use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard,
    },
};

use crate::{
    daemon::MountHostSession,
    mount::{DriveLetter, MountEngine, MountStatus},
};

use super::{
    handle_reservation::{HandleReservation, RenameReservation},
    handle_state::HandleTable,
    DokanFileInfo, DokanyRuntime,
};

pub(super) use super::handle_types::{HandleSnapshot, NodeHandle};

pub(super) struct CallbackContext {
    pub(super) engine: MountEngine,
    pub(super) runtime: DokanyRuntime,
    pub(super) read_only: bool,
    pub(super) case_sensitive_paths: bool,
    pub(super) label: String,
    pub(super) volume_serial: u32,
    pub(super) cache_root_wide: Vec<u16>,
    session: Arc<MountHostSession>,
    handles: HandleTable,
    selected_drive: Mutex<DriveLetter>,
    stop_requested: AtomicBool,
}

impl CallbackContext {
    pub(super) fn new(
        engine: MountEngine,
        runtime: DokanyRuntime,
        session: Arc<MountHostSession>,
        selected_drive: DriveLetter,
        read_only: bool,
        label: String,
        volume_serial: u32,
        cache_root_wide: Vec<u16>,
    ) -> Self {
        let case_sensitive_paths = engine.case_sensitive_paths();
        Self {
            engine,
            runtime,
            read_only,
            case_sensitive_paths,
            label,
            volume_serial,
            cache_root_wide,
            session,
            handles: HandleTable::new(case_sensitive_paths),
            selected_drive: Mutex::new(selected_drive),
            stop_requested: AtomicBool::new(false),
        }
    }

    pub(super) fn reserve_handle(
        &self,
        path: &str,
        is_directory: bool,
        desired_access: u32,
        share_access: u32,
    ) -> io::Result<HandleReservation<'_>> {
        self.handles
            .reserve(path, is_directory, desired_access, share_access)
    }

    pub(super) fn snapshot(&self, key: u64) -> io::Result<HandleSnapshot> {
        self.handles.snapshot(key)
    }

    pub(super) fn cleanup_handle(&self, key: u64) -> io::Result<HandleSnapshot> {
        self.handles.cleanup(key)
    }

    pub(super) fn take(&self, key: u64) -> io::Result<HandleSnapshot> {
        self.handles.take(key)
    }

    pub(super) fn request_delete(
        &self,
        key: u64,
        path: &str,
        is_directory: bool,
    ) -> io::Result<()> {
        self.handles
            .request_delete(&self.engine, key, path, is_directory)
    }

    pub(super) fn cancel_delete(&self, key: u64, path: &str, is_directory: bool) -> io::Result<()> {
        self.handles
            .cancel_delete(&self.engine, key, path, is_directory)
    }

    pub(super) fn commit_delete(&self, key: u64, path: &str, is_directory: bool) -> io::Result<()> {
        self.handles
            .commit_delete(&self.engine, key, path, is_directory)
    }

    pub(super) fn path_matches(&self, left: &str, right: &str) -> bool {
        self.callback_path_key(left) == self.callback_path_key(right)
    }

    pub(super) fn reserve_rename(
        &self,
        key: u64,
        source: &str,
        destination: &str,
        replace_existing: bool,
    ) -> io::Result<RenameReservation<'_>> {
        self.handles
            .reserve_rename(key, source, destination, replace_existing)
    }

    pub(super) fn set_selected_drive(&self, drive: DriveLetter) -> io::Result<()> {
        *self.lock_selected_drive()? = drive;
        Ok(())
    }

    pub(super) fn selected_drive(&self) -> io::Result<DriveLetter> {
        Ok(*self.lock_selected_drive()?)
    }

    pub(super) fn report(&self, status: MountStatus) {
        if self.session.report_status(status).is_err() {
            self.stop_requested.store(true, Ordering::Release);
        }
    }

    pub(super) fn request_stop(&self) {
        self.stop_requested.store(true, Ordering::Release);
    }

    pub(super) fn stop_requested(&self) -> bool {
        self.stop_requested.load(Ordering::Acquire)
    }

    fn lock_selected_drive(&self) -> io::Result<MutexGuard<'_, DriveLetter>> {
        self.selected_drive
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "drive state is unavailable"))
    }

    fn callback_path_key(&self, path: &str) -> String {
        callback_path_key(path, self.case_sensitive_paths)
    }
}

fn callback_path_key(path: &str, case_sensitive: bool) -> String {
    let normalized = path.replace('/', "\\");
    let trimmed = normalized.trim_end_matches('\\');
    let normalized = if trimmed.is_empty() && !normalized.is_empty() {
        "\\".to_string()
    } else {
        trimmed.to_string()
    };
    if case_sensitive {
        normalized
    } else {
        crate::mount::windows_ordinal_key(&normalized)
    }
}

pub(super) unsafe fn context_from_file_info<'a>(
    file_info: *mut DokanFileInfo,
) -> io::Result<&'a CallbackContext> {
    let info = unsafe { file_info.as_ref() }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing Dokany file info"))?;
    let options = unsafe { info.dokan_options.as_ref() }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing Dokany options"))?;
    if options.global_context == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "missing mount callback context",
        ));
    }
    let context = options.global_context as *const CallbackContext;
    unsafe { context.as_ref() }.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid mount callback context",
        )
    })
}

pub(super) unsafe fn context_key(file_info: *mut DokanFileInfo) -> io::Result<u64> {
    let info = unsafe { file_info.as_ref() }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing Dokany file info"))?;
    if info.context == 0 {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "operation has no open file handle",
        ));
    }
    Ok(info.context)
}

pub(super) unsafe fn set_context_key(
    file_info: *mut DokanFileInfo,
    key: u64,
    is_directory: bool,
) -> io::Result<()> {
    let info = unsafe { file_info.as_mut() }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing Dokany file info"))?;
    info.context = key;
    info.is_directory = u8::from(is_directory);
    Ok(())
}
