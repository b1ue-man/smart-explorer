//! Single instance of the se Desktop shell on Windows: the named mutex
//! `Local\SmartExplorerDesktopShell` (C5.5), the daemon's pattern with an own
//! name, so the shell never collides with `Local\SmartExplorerSyncDaemon`.
//! Debug builds append the namespace of isolated end-to-end tests exactly as
//! the daemon does.
use std::borrow::Cow;
use std::os::windows::ffi::OsStrExt;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, OpenMutexW, ReleaseMutex, WaitForSingleObject, MUTEX_MODIFY_STATE,
    SYNCHRONIZATION_SYNCHRONIZE,
};

use super::super::store::DesktopDir;

const MUTEX_NAME: &str = r"Local\SmartExplorerDesktopShell";

#[cfg(debug_assertions)]
#[allow(clippy::duplicate_mod)]
#[path = "../../../windows_test_namespace.rs"]
mod test_namespace;

/// Ownership of the shell mutex; released when dropped.
pub(crate) struct ShellInstance(HANDLE);

impl Drop for ShellInstance {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

#[cfg(not(debug_assertions))]
fn mutex_name() -> Result<Cow<'static, str>, String> {
    Ok(Cow::Borrowed(MUTEX_NAME))
}

#[cfg(debug_assertions)]
fn mutex_name() -> Result<Cow<'static, str>, String> {
    let namespace = test_namespace::from_env()?;
    test_namespace::qualify(MUTEX_NAME, ".Test.", namespace.as_deref()).map(Cow::Owned)
}

fn wide(name: &str) -> Vec<u16> {
    std::ffi::OsStr::new(name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Takes the shell mutex, waiting up to `wait` (a status probe may hold it
/// for a moment). `Ok(None)`: another shell runs.
pub(crate) fn acquire(_dir: &DesktopDir, wait: Duration) -> Result<Option<ShellInstance>, String> {
    let name = wide(&mutex_name()?);
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let milliseconds = wait.as_millis().min(u128::from(u32::MAX - 1)) as u32;
    match unsafe { WaitForSingleObject(handle, milliseconds) } {
        WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Some(ShellInstance(handle))),
        WAIT_TIMEOUT => {
            unsafe { CloseHandle(handle) };
            Ok(None)
        }
        _ => {
            let error = std::io::Error::last_os_error();
            unsafe { CloseHandle(handle) };
            Err(error.to_string())
        }
    }
}

/// Whether a shell owns the mutex. Opening an existing mutex is not
/// ownership: the probe takes it for an instant when nobody owns it.
pub(crate) fn is_running(_dir: &DesktopDir) -> bool {
    let Ok(name) = mutex_name() else {
        return false;
    };
    let name = wide(&name);
    let handle = unsafe {
        OpenMutexW(
            SYNCHRONIZATION_SYNCHRONIZE | MUTEX_MODIFY_STATE,
            0,
            name.as_ptr(),
        )
    };
    if handle.is_null() {
        return false;
    }
    let owned_elsewhere = match unsafe { WaitForSingleObject(handle, 0) } {
        WAIT_OBJECT_0 | WAIT_ABANDONED => {
            unsafe { ReleaseMutex(handle) };
            false
        }
        WAIT_TIMEOUT => true,
        _ => false,
    };
    unsafe { CloseHandle(handle) };
    owned_elsewhere
}
