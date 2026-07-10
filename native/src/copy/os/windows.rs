use std::ffi::c_void;
use std::fs::{File, Metadata};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

pub(super) type FileIdentity = (u32, u64);

#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}

#[repr(C)]
#[derive(Default)]
struct FileInformation {
    attributes: u32,
    creation_time: FileTime,
    last_access_time: FileTime,
    last_write_time: FileTime,
    volume_serial_number: u32,
    file_size_high: u32,
    file_size_low: u32,
    number_of_links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

#[link(name = "kernel32")]
extern "system" {
    #[link_name = "GetFileInformationByHandle"]
    fn get_file_information_by_handle(
        handle: *mut c_void,
        information: *mut FileInformation,
    ) -> i32;
}

pub(super) fn same_file(left: &Path, right: &Path) -> io::Result<bool> {
    let left = File::open(left)?;
    let right = File::open(right)?;
    Ok(file_identity(&left)? == file_identity(&right)?)
}

pub(super) fn file_identity(file: &File) -> io::Result<FileIdentity> {
    let mut information = FileInformation::default();
    let ok =
        unsafe { get_file_information_by_handle(file.as_raw_handle(), &mut information as *mut _) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let index =
        (u64::from(information.file_index_high) << 32) | u64::from(information.file_index_low);
    Ok((information.volume_serial_number, index))
}

pub(super) fn path_matches_identity(path: &Path, expected: FileIdentity) -> io::Result<bool> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    Ok(file_identity(&file)? == expected)
}

pub(super) fn metadata_is_link_like(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

pub(super) fn path_text(path: &Path) -> io::Result<String> {
    path.to_str()
        .map(|path| path.replace('\\', "/"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "path is not valid Unicode"))
}

pub(super) fn path_key(path: &Path) -> io::Result<String> {
    path_text(path).map(|path| path.to_lowercase())
}

pub(super) fn commit_staged(staged: &Path, dest: &Path, overwrite: bool) -> io::Result<()> {
    if overwrite {
        replace_file_atomic(staged, dest)
    } else {
        move_file_no_replace(staged, dest)
    }
}

pub(super) fn move_file(src: &Path, dest: &Path, overwrite: bool) -> io::Result<()> {
    if overwrite {
        replace_file_atomic(src, dest)
    } else {
        move_file_no_replace(src, dest)
    }
}

fn move_file_no_replace(src: &Path, dest: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH;
    move_file_ex(src, dest, MOVEFILE_WRITE_THROUGH)
}

fn replace_file_atomic(src: &Path, dest: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    move_file_ex(
        src,
        dest,
        MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
    )
}

fn move_file_ex(src: &Path, dest: &Path, flags: u32) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
    let src: Vec<u16> = src.as_os_str().encode_wide().chain(Some(0)).collect();
    let dest: Vec<u16> = dest.as_os_str().encode_wide().chain(Some(0)).collect();
    let ok = unsafe { MoveFileExW(src.as_ptr(), dest.as_ptr(), flags) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(super) fn is_cross_device(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::CrossesDevices || error.raw_os_error() == Some(17)
}

pub(super) fn sync_parent(_path: &Path) -> io::Result<()> {
    // MOVEFILE_WRITE_THROUGH is used by replacement commits. Opening a
    // directory for FlushFileBuffers requires extra Win32 privileges and is
    // not needed for the no-replace MoveFile path.
    Ok(())
}
