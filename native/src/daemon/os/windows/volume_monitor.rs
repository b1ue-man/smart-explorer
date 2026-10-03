//! Register arrivals before enumerating; callback only wakes this thread.
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{CM_Register_Notification, CM_Unregister_Notification, CM_NOTIFY_FILTER, CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE, CM_NOTIFY_ACTION, CM_NOTIFY_EVENT_DATA, HCMNOTIFICATION, CR_SUCCESS};
use windows_sys::Win32::System::Ioctl::GUID_DEVINTERFACE_VOLUME;
use super::DriveInfo;
static STATE: OnceLock<Arc<Mutex<Option<Vec<DriveInfo>>>>> = OnceLock::new();
pub(super) fn snapshot() -> Option<Vec<DriveInfo>> {
    let state = STATE.get_or_init(|| {
        let state = Arc::new(Mutex::new(None)); let worker = state.clone();
        let _ = std::thread::Builder::new().name("sync-volume-events".into()).spawn(move || monitor(worker));
        state
    });
    state.lock().unwrap_or_else(PoisonError::into_inner).clone()
}
fn monitor(state: Arc<Mutex<Option<Vec<DriveInfo>>>>) {
    let (sender, receiver) = crossbeam_channel::bounded::<()>(1);
    let context = Box::into_raw(Box::new(sender));
    let mut filter: CM_NOTIFY_FILTER = unsafe { std::mem::zeroed() };
    filter.cbSize = std::mem::size_of_val(&filter) as u32; filter.FilterType = CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE;
    // SAFETY: the selected filter type makes this the active union member.
    unsafe { filter.u.DeviceInterface.ClassGuid = GUID_DEVINTERFACE_VOLUME; }
    let mut notification = std::ptr::null_mut();
    let registered = unsafe { CM_Register_Notification(&filter, context.cast(), Some(arrived), &mut notification) } == CR_SUCCESS;
    if !registered { crate::daemon::log_worker("Volume-Ereignisse nicht verfügbar; Anschluss-Erkennung fragt alle fünf Sekunden ab."); }
    loop {
        let drives = super::drives::removable();
        *state.lock().unwrap_or_else(PoisonError::into_inner) = Some(drives);
        match receiver.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(()) | Err(crossbeam_channel::RecvTimeoutError::Timeout) => {},
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }
    if registered { unsafe { CM_Unregister_Notification(notification); } }
    unsafe { drop(Box::from_raw(context)); }
}
unsafe extern "system" fn arrived(_: HCMNOTIFICATION, context: *const std::ffi::c_void,
    _: CM_NOTIFY_ACTION, _: *const CM_NOTIFY_EVENT_DATA, _: u32) -> u32 {
    let sender = unsafe { &*context.cast::<crossbeam_channel::Sender<()>>() };
    let _ = sender.try_send(()); 0
}
