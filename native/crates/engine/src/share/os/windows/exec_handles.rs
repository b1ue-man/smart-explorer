//! Owned Win32 handles and process-thread attribute lists. The supervisor
//! launcher in `exec.rs` and the ConPTY launcher in `exec_conpty.rs` share them.
use std::io;
use std::mem::size_of;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Threading::{
    DeleteProcThreadAttributeList, InitializeProcThreadAttributeList, UpdateProcThreadAttribute,
    LPPROC_THREAD_ATTRIBUTE_LIST,
};

pub(super) struct AttributeList {
    bytes: Vec<usize>,
    initialized: bool,
}

impl AttributeList {
    pub(super) fn new(count: u32) -> io::Result<Self> {
        let mut size = 0usize;
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), count, 0, &mut size) };
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        let words = size.div_ceil(size_of::<usize>());
        let mut value = Self {
            bytes: vec![0; words],
            initialized: false,
        };
        if unsafe { InitializeProcThreadAttributeList(value.ptr(), count, 0, &mut size) } == 0 {
            return Err(io::Error::last_os_error());
        }
        value.initialized = true;
        Ok(value)
    }

    pub(super) fn ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.bytes.as_mut_ptr().cast()
    }

    pub(super) fn set(
        &mut self,
        attribute: usize,
        value: *const core::ffi::c_void,
        size: usize,
    ) -> io::Result<()> {
        if unsafe {
            UpdateProcThreadAttribute(
                self.ptr(),
                0,
                attribute,
                value,
                size,
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        } == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { DeleteProcThreadAttributeList(self.ptr()) };
        }
    }
}

pub(super) struct OwnedHandle(HANDLE);

// SAFETY: a kernel handle belongs to the process, not to a thread; the
// account service hands pipe and Job handles to the thread that serves them.
unsafe impl Send for OwnedHandle {}

impl OwnedHandle {
    pub(super) fn new(handle: HANDLE) -> io::Result<Self> {
        if handle.is_null() {
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
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}
