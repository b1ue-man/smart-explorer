use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::process::ExitStatusExt;
use std::process::ExitStatus;

use windows_sys::Win32::Foundation::{
    CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, TerminateProcess, WaitForSingleObject, INFINITE,
};

/// Closes over every mount-host process when the daemon exits. The process is
/// assigned by CreateProcess itself; there is no running uncontained window.
pub(super) struct MountHostJob(OwnedHandle);

impl MountHostJob {
    pub(super) fn create() -> io::Result<Self> {
        let job = Self(OwnedHandle::new(unsafe {
            CreateJobObjectW(std::ptr::null(), std::ptr::null())
        })?);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let changed = unsafe {
            SetInformationJobObject(
                job.raw(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if changed == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    pub(super) fn raw(&self) -> HANDLE {
        self.0.raw()
    }
}

pub(super) struct MountHostChild(OwnedHandle);

impl MountHostChild {
    pub(super) fn from_owned(process: OwnedHandle) -> Self {
        Self(process)
    }

    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match unsafe { WaitForSingleObject(self.0.raw(), 0) } {
            WAIT_OBJECT_0 => self.exit_status().map(Some),
            WAIT_TIMEOUT => Ok(None),
            WAIT_FAILED => Err(io::Error::last_os_error()),
            result => Err(io::Error::other(format!(
                "unexpected mount-host wait result: {result}"
            ))),
        }
    }

    pub(super) fn wait(&mut self) -> io::Result<ExitStatus> {
        match unsafe { WaitForSingleObject(self.0.raw(), INFINITE) } {
            WAIT_OBJECT_0 => self.exit_status(),
            WAIT_FAILED => Err(io::Error::last_os_error()),
            result => Err(io::Error::other(format!(
                "unexpected mount-host wait result: {result}"
            ))),
        }
    }

    pub(super) fn kill(&mut self) -> io::Result<()> {
        if self.try_wait()?.is_some() {
            return Ok(());
        }
        if unsafe { TerminateProcess(self.0.raw(), 1) } != 0 {
            return Ok(());
        }
        let terminate_error = io::Error::last_os_error();
        self.try_wait()?.map_or(Err(terminate_error), |_| Ok(()))
    }

    fn exit_status(&self) -> io::Result<ExitStatus> {
        let mut code = 0u32;
        if unsafe { GetExitCodeProcess(self.0.raw(), &mut code) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(ExitStatus::from_raw(code))
        }
    }
}

pub(super) struct OwnedHandle(HANDLE);

// Kernel handles are valid across threads; ownership remains unique here.
unsafe impl Send for OwnedHandle {}

impl OwnedHandle {
    pub(super) fn new(handle: HANDLE) -> io::Result<Self> {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(handle))
        }
    }

    pub(super) fn raw(&self) -> HANDLE {
        self.0
    }

    pub(super) fn into_raw(mut self) -> HANDLE {
        let raw = self.0;
        self.0 = std::ptr::null_mut();
        raw
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.0) };
        }
    }
}
