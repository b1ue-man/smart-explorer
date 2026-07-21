use std::ffi::c_void;

use windows_sys::Win32::Foundation::{FILETIME, STATUS_NOT_IMPLEMENTED};

use super::{
    callback_status::{guard_with_context, unsupported, CallbackFailure},
    dokany_abi::FillFindStreamData,
    DokanFileInfo, NtStatus,
};

pub(super) unsafe extern "system" fn set_file_attributes(
    _file_name: *const u16,
    _attributes: u32,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe { guard_with_context(file_info, |_context| Err(unsupported())) }
}

pub(super) unsafe extern "system" fn set_file_time(
    _file_name: *const u16,
    _creation: *const FILETIME,
    _access: *const FILETIME,
    _write: *const FILETIME,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe { guard_with_context(file_info, |_context| Err(unsupported())) }
}

pub(super) unsafe extern "system" fn get_file_security(
    _file_name: *const u16,
    _security_information: *mut u32,
    _security_descriptor: *mut c_void,
    _buffer_length: u32,
    _length_needed: *mut u32,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    // STATUS_NOT_IMPLEMENTED specifically asks Dokany to synthesize the
    // current-user descriptor. STATUS_NOT_SUPPORTED would instead make common
    // Explorer/application security probes fail outright.
    unsafe {
        guard_with_context(file_info, |_context| {
            Err(CallbackFailure::Nt(STATUS_NOT_IMPLEMENTED))
        })
    }
}

pub(super) unsafe extern "system" fn set_file_security(
    _file_name: *const u16,
    _security_information: *mut u32,
    _security_descriptor: *mut c_void,
    _buffer_length: u32,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe { guard_with_context(file_info, |_context| Err(unsupported())) }
}

pub(super) unsafe extern "system" fn find_streams(
    _file_name: *const u16,
    _fill: FillFindStreamData,
    _find_stream_context: *mut c_void,
    file_info: *mut DokanFileInfo,
) -> NtStatus {
    unsafe { guard_with_context(file_info, |_context| Err(unsupported())) }
}
