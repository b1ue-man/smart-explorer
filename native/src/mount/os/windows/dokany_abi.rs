//! Minimal Dokany 2.3.1 (library API 231, driver protocol `0x190`) ABI used by
//! the dynamic runtime loader.
//!
//! This is intentionally not a link-time binding: linking `dokan2.lib` would
//! prevent every Smart Explorer executable from starting when Dokany is not
//! installed. The layout and signatures below were audited against
//! `dokan-dev/dokany` tag `v2.3.1.1000` (`dokan/dokan.h`) and the MIT-licensed
//! `dokan-dev/dokan-rust` source at commit
//! `ce2ec8565591d21ed5b58f8233e9d81a730823ad`. The latter targets API 230;
//! Dokany's tagged 2.3.1 `dokan/dokan.h` is the source of truth for library API
//! 231, while `sys/public.h` defines the independent driver protocol as
//! `0x190` (decimal 400).

use std::ffi::c_void;

use windows_sys::Win32::{
    Foundation::FILETIME,
    Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, WIN32_FIND_DATAW, WIN32_FIND_STREAM_DATA},
};

pub(crate) const DOKANY_LIBRARY_API_VERSION: u32 = crate::mount::DOKANY_LIBRARY_API_VERSION;
// Retained for the pinned-manifest verifier and existing Windows adapter API.
pub(crate) const DOKANY_API_VERSION: u32 = DOKANY_LIBRARY_API_VERSION;
pub(crate) const DOKANY_DRIVER_PROTOCOL_VERSION: u32 =
    crate::mount::DOKANY_DRIVER_PROTOCOL_VERSION;
pub(crate) const DOKANY_DLL_NAME: &str = "dokan2.dll";
pub(crate) const VOLUME_SECURITY_DESCRIPTOR_MAX_SIZE: usize = 16 * 1024;

pub(crate) const OPTION_WRITE_PROTECT: u32 = 1 << 3;
pub(crate) const OPTION_MOUNT_MANAGER: u32 = 1 << 6;
pub(crate) const OPTION_CURRENT_SESSION: u32 = 1 << 7;
pub(crate) const OPTION_CASE_SENSITIVE: u32 = 1 << 9;
pub(crate) const OPTION_ALLOW_IPC_BATCHING: u32 = 1 << 12;

pub(crate) const SUCCESS: i32 = 0;
pub(crate) const ERROR: i32 = -1;
pub(crate) const DRIVE_LETTER_ERROR: i32 = -2;
pub(crate) const DRIVER_INSTALL_ERROR: i32 = -3;
pub(crate) const START_ERROR: i32 = -4;
pub(crate) const MOUNT_ERROR: i32 = -5;
pub(crate) const MOUNT_POINT_ERROR: i32 = -6;
pub(crate) const VERSION_ERROR: i32 = -7;

pub(crate) type NtStatus = i32;
pub(crate) type DokanHandle = *mut c_void;

#[repr(C)]
pub(crate) struct DokanOptions {
    pub(crate) version: u16,
    pub(crate) single_thread: u8,
    pub(crate) options: u32,
    pub(crate) global_context: u64,
    pub(crate) mount_point: *const u16,
    pub(crate) unc_name: *const u16,
    pub(crate) timeout: u32,
    pub(crate) allocation_unit_size: u32,
    pub(crate) sector_size: u32,
    pub(crate) volume_security_descriptor_length: u32,
    pub(crate) volume_security_descriptor: [i8; VOLUME_SECURITY_DESCRIPTOR_MAX_SIZE],
}

impl Default for DokanOptions {
    fn default() -> Self {
        Self {
            version: DOKANY_LIBRARY_API_VERSION as u16,
            single_thread: 0,
            options: 0,
            global_context: 0,
            mount_point: std::ptr::null(),
            unc_name: std::ptr::null(),
            timeout: 0,
            allocation_unit_size: 0,
            sector_size: 0,
            volume_security_descriptor_length: 0,
            volume_security_descriptor: [0; VOLUME_SECURITY_DESCRIPTOR_MAX_SIZE],
        }
    }
}

#[repr(C)]
pub(crate) struct DokanFileInfo {
    pub(crate) context: u64,
    pub(crate) dokan_context: u64,
    pub(crate) dokan_options: *mut DokanOptions,
    pub(crate) processing_context: *mut c_void,
    pub(crate) process_id: u32,
    pub(crate) is_directory: u8,
    pub(crate) delete_pending: u8,
    pub(crate) paging_io: u8,
    pub(crate) synchronous_io: u8,
    pub(crate) no_cache: u8,
    pub(crate) write_to_end_of_file: u8,
}

/// Only passed through to `ZwCreateFile`; Smart Explorer never dereferences it.
pub(crate) type DokanIoSecurityContext = c_void;

pub(crate) type FillFindData = unsafe extern "system" fn(
    find_data: *mut WIN32_FIND_DATAW,
    file_info: *mut DokanFileInfo,
) -> i32;
pub(crate) type FillFindStreamData =
    unsafe extern "system" fn(find_data: *mut WIN32_FIND_STREAM_DATA, context: *mut c_void) -> i32;

pub(crate) type CreateFileCallback = unsafe extern "system" fn(
    file_name: *const u16,
    security_context: *mut DokanIoSecurityContext,
    desired_access: u32,
    file_attributes: u32,
    share_access: u32,
    create_disposition: u32,
    create_options: u32,
    file_info: *mut DokanFileInfo,
) -> NtStatus;
pub(crate) type FileCallback = unsafe extern "system" fn(*const u16, *mut DokanFileInfo);
pub(crate) type ReadFileCallback = unsafe extern "system" fn(
    *const u16,
    *mut c_void,
    u32,
    *mut u32,
    i64,
    *mut DokanFileInfo,
) -> NtStatus;
pub(crate) type WriteFileCallback = unsafe extern "system" fn(
    *const u16,
    *const c_void,
    u32,
    *mut u32,
    i64,
    *mut DokanFileInfo,
) -> NtStatus;
pub(crate) type StatusFileCallback =
    unsafe extern "system" fn(*const u16, *mut DokanFileInfo) -> NtStatus;
pub(crate) type GetFileInformationCallback = unsafe extern "system" fn(
    *const u16,
    *mut BY_HANDLE_FILE_INFORMATION,
    *mut DokanFileInfo,
) -> NtStatus;
pub(crate) type FindFilesCallback =
    unsafe extern "system" fn(*const u16, FillFindData, *mut DokanFileInfo) -> NtStatus;
pub(crate) type FindFilesWithPatternCallback =
    unsafe extern "system" fn(*const u16, *const u16, FillFindData, *mut DokanFileInfo) -> NtStatus;
pub(crate) type SetFileAttributesCallback =
    unsafe extern "system" fn(*const u16, u32, *mut DokanFileInfo) -> NtStatus;
pub(crate) type SetFileTimeCallback = unsafe extern "system" fn(
    *const u16,
    *const FILETIME,
    *const FILETIME,
    *const FILETIME,
    *mut DokanFileInfo,
) -> NtStatus;
pub(crate) type MoveFileCallback =
    unsafe extern "system" fn(*const u16, *const u16, i32, *mut DokanFileInfo) -> NtStatus;
pub(crate) type SetSizeCallback =
    unsafe extern "system" fn(*const u16, i64, *mut DokanFileInfo) -> NtStatus;
pub(crate) type LockFileCallback =
    unsafe extern "system" fn(*const u16, i64, i64, *mut DokanFileInfo) -> NtStatus;
pub(crate) type GetDiskFreeSpaceCallback =
    unsafe extern "system" fn(*mut u64, *mut u64, *mut u64, *mut DokanFileInfo) -> NtStatus;
pub(crate) type GetVolumeInformationCallback = unsafe extern "system" fn(
    *mut u16,
    u32,
    *mut u32,
    *mut u32,
    *mut u32,
    *mut u16,
    u32,
    *mut DokanFileInfo,
) -> NtStatus;
pub(crate) type MountedCallback =
    unsafe extern "system" fn(*const u16, *mut DokanFileInfo) -> NtStatus;
pub(crate) type UnmountedCallback = unsafe extern "system" fn(*mut DokanFileInfo) -> NtStatus;
pub(crate) type GetFileSecurityCallback = unsafe extern "system" fn(
    *const u16,
    *mut u32,
    *mut c_void,
    u32,
    *mut u32,
    *mut DokanFileInfo,
) -> NtStatus;
pub(crate) type SetFileSecurityCallback = unsafe extern "system" fn(
    *const u16,
    *mut u32,
    *mut c_void,
    u32,
    *mut DokanFileInfo,
) -> NtStatus;
pub(crate) type FindStreamsCallback = unsafe extern "system" fn(
    *const u16,
    FillFindStreamData,
    *mut c_void,
    *mut DokanFileInfo,
) -> NtStatus;

#[repr(C)]
#[derive(Clone, Default)]
pub(crate) struct DokanOperations {
    pub(crate) create_file: Option<CreateFileCallback>,
    pub(crate) cleanup: Option<FileCallback>,
    pub(crate) close_file: Option<FileCallback>,
    pub(crate) read_file: Option<ReadFileCallback>,
    pub(crate) write_file: Option<WriteFileCallback>,
    pub(crate) flush_file_buffers: Option<StatusFileCallback>,
    pub(crate) get_file_information: Option<GetFileInformationCallback>,
    pub(crate) find_files: Option<FindFilesCallback>,
    pub(crate) find_files_with_pattern: Option<FindFilesWithPatternCallback>,
    pub(crate) set_file_attributes: Option<SetFileAttributesCallback>,
    pub(crate) set_file_time: Option<SetFileTimeCallback>,
    pub(crate) delete_file: Option<StatusFileCallback>,
    pub(crate) delete_directory: Option<StatusFileCallback>,
    pub(crate) move_file: Option<MoveFileCallback>,
    pub(crate) set_end_of_file: Option<SetSizeCallback>,
    pub(crate) set_allocation_size: Option<SetSizeCallback>,
    pub(crate) lock_file: Option<LockFileCallback>,
    pub(crate) unlock_file: Option<LockFileCallback>,
    pub(crate) get_disk_free_space: Option<GetDiskFreeSpaceCallback>,
    pub(crate) get_volume_information: Option<GetVolumeInformationCallback>,
    pub(crate) mounted: Option<MountedCallback>,
    pub(crate) unmounted: Option<UnmountedCallback>,
    pub(crate) get_file_security: Option<GetFileSecurityCallback>,
    pub(crate) set_file_security: Option<SetFileSecurityCallback>,
    pub(crate) find_streams: Option<FindStreamsCallback>,
}
