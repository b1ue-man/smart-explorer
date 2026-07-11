use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::os::windows::io::AsRawHandle;

pub(super) fn local_path(path: &str) -> std::path::PathBuf {
    let rooted;
    let bytes = path.as_bytes();
    let path = if bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        rooted = format!("{path}/");
        rooted.as_str()
    } else {
        path
    };
    std::path::PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR))
}

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

pub(super) fn same_file(left: &str, right: &str) -> io::Result<bool> {
    // Keep both handles open while comparing their indexes. Windows may reuse
    // an index after the corresponding handle closes.
    let left = File::open(left)?;
    let right = File::open(right)?;
    Ok(file_key(&left)? == file_key(&right)?)
}

fn file_key(file: &File) -> io::Result<(u32, u64)> {
    let mut information = FileInformation::default();
    // SAFETY: `file` owns a valid handle for this call, and `information` is a
    // writable, correctly laid-out output structure that lives until it returns.
    let ok =
        unsafe { get_file_information_by_handle(file.as_raw_handle(), &mut information as *mut _) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // The legacy index can collide on ReFS; the only consequence here is a
    // conservative refusal to copy, never a missed self-target check.
    let index =
        (u64::from(information.file_index_high) << 32) | u64::from(information.file_index_low);
    Ok((information.volume_serial_number, index))
}

pub(super) fn validate_connection_protocol(
    _protocol: crate::creds::Protocol,
) -> Result<(), String> {
    Ok(())
}
