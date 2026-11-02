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

#[cfg(test)]
#[path = "mount_options_task_tests.rs"]
mod mount_batching_task;

use std::ffi::c_void;

use windows_sys::Win32::{
    Foundation::FILETIME,
    Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, WIN32_FIND_DATAW, WIN32_FIND_STREAM_DATA},
};

pub const DOKANY_LIBRARY_API_VERSION: u32 = crate::mount::DOKANY_LIBRARY_API_VERSION;
// Retained for the pinned-manifest verifier and existing Windows adapter API.
pub const DOKANY_API_VERSION: u32 = DOKANY_LIBRARY_API_VERSION;
pub const DOKANY_DRIVER_PROTOCOL_VERSION: u32 = crate::mount::DOKANY_DRIVER_PROTOCOL_VERSION;
pub const DOKANY_DLL_NAME: &str = "dokan2.dll";
pub const VOLUME_SECURITY_DESCRIPTOR_MAX_SIZE: usize = 16 * 1024;

pub const OPTION_WRITE_PROTECT: u32 = 1 << 3;
pub const OPTION_MOUNT_MANAGER: u32 = 1 << 6;
pub const OPTION_CURRENT_SESSION: u32 = 1 << 7;
pub const OPTION_CASE_SENSITIVE: u32 = 1 << 9;
pub const OPTION_ALLOW_IPC_BATCHING: u32 = 1 << 12;

pub const SUCCESS: i32 = 0;
pub const ERROR: i32 = -1;
pub const DRIVE_LETTER_ERROR: i32 = -2;
pub const DRIVER_INSTALL_ERROR: i32 = -3;
pub const START_ERROR: i32 = -4;
pub const MOUNT_ERROR: i32 = -5;
pub const MOUNT_POINT_ERROR: i32 = -6;
pub const VERSION_ERROR: i32 = -7;

pub type NtStatus = i32;
pub type DokanHandle = *mut c_void;

#[repr(C)]
pub struct DokanOptions {
    pub version: u16,
    pub single_thread: u8,
    pub options: u32,
    pub global_context: u64,
    pub mount_point: *const u16,
    pub unc_name: *const u16,
    pub timeout: u32,
    pub allocation_unit_size: u32,
    pub sector_size: u32,
    pub volume_security_descriptor_length: u32,
    pub volume_security_descriptor: [i8; VOLUME_SECURITY_DESCRIPTOR_MAX_SIZE],
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

impl DokanOptions {
    pub(super) fn prepare_for_create(&mut self) {
        // Dokany 2.3.1 sends batching to the driver but truncates 0x1000 to
        // BOOLEAN (u8) when choosing its consumer, dropping trailing events.
        // Clear this BEFORE creation; the DLL can mutate/reuse these options.
        // Keep normal parallel workers and every unrelated option unchanged.
        // Evidence and the condition for removing this guard: docs/MOUNT_BATCHING.md.
        self.options &= !OPTION_ALLOW_IPC_BATCHING;
    }
}

#[repr(C)]
pub struct DokanFileInfo {
    pub context: u64,
    pub dokan_context: u64,
    pub dokan_options: *mut DokanOptions,
    pub processing_context: *mut c_void,
    pub process_id: u32,
    pub is_directory: u8,
    pub delete_pending: u8,
    pub paging_io: u8,
    pub synchronous_io: u8,
    pub no_cache: u8,
    pub write_to_end_of_file: u8,
}

/// Only passed through to `ZwCreateFile`; Smart Explorer never dereferences it.
pub type DokanIoSecurityContext = c_void;

pub type FillFindData = unsafe extern "system" fn(
    find_data: *mut WIN32_FIND_DATAW,
    file_info: *mut DokanFileInfo,
) -> i32;
pub type FillFindStreamData =
    unsafe extern "system" fn(find_data: *mut WIN32_FIND_STREAM_DATA, context: *mut c_void) -> i32;

pub type CreateFileCallback = unsafe extern "system" fn(
    file_name: *const u16,
    security_context: *mut DokanIoSecurityContext,
    desired_access: u32,
    file_attributes: u32,
    share_access: u32,
    create_disposition: u32,
    create_options: u32,
    file_info: *mut DokanFileInfo,
) -> NtStatus;
pub type FileCallback = unsafe extern "system" fn(*const u16, *mut DokanFileInfo);
pub type ReadFileCallback = unsafe extern "system" fn(
    *const u16,
    *mut c_void,
    u32,
    *mut u32,
    i64,
    *mut DokanFileInfo,
) -> NtStatus;
pub type WriteFileCallback = unsafe extern "system" fn(
    *const u16,
    *const c_void,
    u32,
    *mut u32,
    i64,
    *mut DokanFileInfo,
) -> NtStatus;
pub type StatusFileCallback =
    unsafe extern "system" fn(*const u16, *mut DokanFileInfo) -> NtStatus;
pub type GetFileInformationCallback = unsafe extern "system" fn(
    *const u16,
    *mut BY_HANDLE_FILE_INFORMATION,
    *mut DokanFileInfo,
) -> NtStatus;
pub type FindFilesCallback =
    unsafe extern "system" fn(*const u16, FillFindData, *mut DokanFileInfo) -> NtStatus;
pub type FindFilesWithPatternCallback =
    unsafe extern "system" fn(*const u16, *const u16, FillFindData, *mut DokanFileInfo) -> NtStatus;
pub type SetFileAttributesCallback =
    unsafe extern "system" fn(*const u16, u32, *mut DokanFileInfo) -> NtStatus;
pub type SetFileTimeCallback = unsafe extern "system" fn(
    *const u16,
    *const FILETIME,
    *const FILETIME,
    *const FILETIME,
    *mut DokanFileInfo,
) -> NtStatus;
pub type MoveFileCallback =
    unsafe extern "system" fn(*const u16, *const u16, i32, *mut DokanFileInfo) -> NtStatus;
pub type SetSizeCallback =
    unsafe extern "system" fn(*const u16, i64, *mut DokanFileInfo) -> NtStatus;
pub type LockFileCallback =
    unsafe extern "system" fn(*const u16, i64, i64, *mut DokanFileInfo) -> NtStatus;
pub type GetDiskFreeSpaceCallback =
    unsafe extern "system" fn(*mut u64, *mut u64, *mut u64, *mut DokanFileInfo) -> NtStatus;
pub type GetVolumeInformationCallback = unsafe extern "system" fn(
    *mut u16,
    u32,
    *mut u32,
    *mut u32,
    *mut u32,
    *mut u16,
    u32,
    *mut DokanFileInfo,
) -> NtStatus;
pub type MountedCallback =
    unsafe extern "system" fn(*const u16, *mut DokanFileInfo) -> NtStatus;
pub type UnmountedCallback = unsafe extern "system" fn(*mut DokanFileInfo) -> NtStatus;
pub type GetFileSecurityCallback = unsafe extern "system" fn(
    *const u16,
    *mut u32,
    *mut c_void,
    u32,
    *mut u32,
    *mut DokanFileInfo,
) -> NtStatus;
pub type SetFileSecurityCallback = unsafe extern "system" fn(
    *const u16,
    *mut u32,
    *mut c_void,
    u32,
    *mut DokanFileInfo,
) -> NtStatus;
pub type FindStreamsCallback = unsafe extern "system" fn(
    *const u16,
    FillFindStreamData,
    *mut c_void,
    *mut DokanFileInfo,
) -> NtStatus;

#[repr(C)]
#[derive(Clone, Default)]
pub struct DokanOperations {
    pub create_file: Option<CreateFileCallback>,
    pub cleanup: Option<FileCallback>,
    pub close_file: Option<FileCallback>,
    pub read_file: Option<ReadFileCallback>,
    pub write_file: Option<WriteFileCallback>,
    pub flush_file_buffers: Option<StatusFileCallback>,
    pub get_file_information: Option<GetFileInformationCallback>,
    pub find_files: Option<FindFilesCallback>,
    pub find_files_with_pattern: Option<FindFilesWithPatternCallback>,
    pub set_file_attributes: Option<SetFileAttributesCallback>,
    pub set_file_time: Option<SetFileTimeCallback>,
    pub delete_file: Option<StatusFileCallback>,
    pub delete_directory: Option<StatusFileCallback>,
    pub move_file: Option<MoveFileCallback>,
    pub set_end_of_file: Option<SetSizeCallback>,
    pub set_allocation_size: Option<SetSizeCallback>,
    pub lock_file: Option<LockFileCallback>,
    pub unlock_file: Option<LockFileCallback>,
    pub get_disk_free_space: Option<GetDiskFreeSpaceCallback>,
    pub get_volume_information: Option<GetVolumeInformationCallback>,
    pub mounted: Option<MountedCallback>,
    pub unmounted: Option<UnmountedCallback>,
    pub get_file_security: Option<GetFileSecurityCallback>,
    pub set_file_security: Option<SetFileSecurityCallback>,
    pub find_streams: Option<FindStreamsCallback>,
}
