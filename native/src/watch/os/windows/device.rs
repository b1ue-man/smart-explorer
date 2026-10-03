//! Safe removal of watched volumes (Windows, RV1 B08): a directory handle
//! keeps its volume busy, so "Hardware sicher entfernen" would fail while a
//! root is watched. Each local watch registers for device events of its
//! handle (`CM_Register_Notification`, `CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE`);
//! when removal is queried the callback closes the handle at once (the
//! documented response) and tells the watch thread, which reports
//! `Unavailable(DeviceRemoved)` and arms the root again once it is back.

use std::sync::{Arc, Mutex, PoisonError};

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Register_Notification, CM_Unregister_Notification, CM_NOTIFY_ACTION,
    CM_NOTIFY_ACTION_DEVICEQUERYREMOVE, CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE,
    CM_NOTIFY_ACTION_DEVICEREMOVEPENDING, CM_NOTIFY_EVENT_DATA, CM_NOTIFY_FILTER,
    CM_NOTIFY_FILTER_0, CM_NOTIFY_FILTER_0_0, CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE, CR_SUCCESS,
    HCMNOTIFICATION,
};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::IO::{CancelIoEx, PostQueuedCompletionStatus};

/// Completion keys with this bit carry a device event (the action in the byte
/// count) for the watch slot in the other bits.
pub(super) const DEVICE_BIT: usize = 1 << (usize::BITS - 1);

/// A directory handle that the device callback may close from another thread.
pub(super) struct HandleCell(Mutex<usize>);

impl HandleCell {
    pub(super) fn new(handle: HANDLE) -> Self {
        Self(Mutex::new(handle as usize))
    }

    /// Runs `use_handle` with the open handle; `None` once it was closed.
    pub(super) fn with<R>(&self, use_handle: impl FnOnce(HANDLE) -> R) -> Option<R> {
        let guard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        (*guard != 0).then(|| use_handle(*guard as HANDLE))
    }

    /// Cancels pending I/O and closes the handle (once).
    pub(super) fn close(&self) {
        let mut guard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let raw = std::mem::take(&mut *guard);
        if raw != 0 {
            let handle = raw as HANDLE;
            // SAFETY: the handle is open and owned by this cell; closing it
            // completes the pending read with an abort status.
            unsafe {
                CancelIoEx(handle, std::ptr::null());
                CloseHandle(handle);
            }
        }
    }
}

struct Context {
    handle: Arc<HandleCell>,
    port: usize,
    key: usize,
}

/// A live registration; `unregister` must run on a thread that is not the
/// callback (Microsoft: it waits for pending callbacks).
pub(super) struct DeviceWatch {
    notification: usize,
    context: usize,
}

impl DeviceWatch {
    pub(super) fn register(handle: &Arc<HandleCell>, port: HANDLE, key: usize) -> Option<Self> {
        let context = Box::into_raw(Box::new(Context {
            handle: handle.clone(),
            port: port as usize,
            key,
        }));
        let registered = handle.with(|target| {
            // SAFETY: an all-zero filter is valid; the fields used are set.
            let mut filter: CM_NOTIFY_FILTER = unsafe { std::mem::zeroed() };
            filter.cbSize = std::mem::size_of::<CM_NOTIFY_FILTER>() as u32;
            filter.FilterType = CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE;
            filter.u = CM_NOTIFY_FILTER_0 {
                DeviceHandle: CM_NOTIFY_FILTER_0_0 { hTarget: target },
            };
            let mut notification: HCMNOTIFICATION = std::ptr::null_mut();
            // SAFETY: the context outlives the registration (freed after
            // `CM_Unregister_Notification` returns).
            let result = unsafe {
                CM_Register_Notification(
                    &filter,
                    context as *const std::ffi::c_void,
                    Some(on_device),
                    &mut notification,
                )
            };
            (result == CR_SUCCESS).then_some(notification as usize)
        });
        match registered.flatten() {
            Some(notification) => Some(Self {
                notification,
                context: context as usize,
            }),
            None => {
                // SAFETY: never registered, so nothing else holds the pointer.
                drop(unsafe { Box::from_raw(context) });
                None
            }
        }
    }

    pub(super) fn unregister(self) {
        // SAFETY: the registration is live; after the call no callback runs,
        // so the context can be freed.
        unsafe {
            CM_Unregister_Notification(self.notification as HCMNOTIFICATION);
            drop(Box::from_raw(self.context as *mut Context));
        }
    }
}

unsafe extern "system" fn on_device(
    _notification: HCMNOTIFICATION,
    context: *const std::ffi::c_void,
    action: CM_NOTIFY_ACTION,
    _data: *const CM_NOTIFY_EVENT_DATA,
    _size: u32,
) -> u32 {
    // SAFETY: the pointer was registered with this callback and stays valid
    // until the registration is gone.
    let context = unsafe { &*(context as *const Context) };
    if matches!(
        action,
        CM_NOTIFY_ACTION_DEVICEQUERYREMOVE
            | CM_NOTIFY_ACTION_DEVICEREMOVEPENDING
            | CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE
    ) {
        context.handle.close();
    }
    // SAFETY: posts a packet without OVERLAPPED to the live port.
    unsafe {
        PostQueuedCompletionStatus(
            context.port as HANDLE,
            action as u32,
            context.key | DEVICE_BIT,
            std::ptr::null(),
        );
    }
    // ERROR_SUCCESS: never veto the removal.
    0
}
