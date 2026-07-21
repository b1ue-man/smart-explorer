use std::io;

use windows_sys::Win32::{
    Foundation::FILETIME,
    Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, WIN32_FIND_DATAW},
};

use crate::vfs::VfsMeta;

use super::wide::encode_find_name;

const FILE_ATTRIBUTE_READONLY: u32 = 0x0000_0001;
const FILE_ATTRIBUTE_HIDDEN: u32 = 0x0000_0002;
const FILE_ATTRIBUTE_SYSTEM: u32 = 0x0000_0004;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x0000_0020;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x0000_0080;
const WINDOWS_EPOCH_OFFSET_MS: i128 = 11_644_473_600_000;

pub(super) fn file_information(
    meta: &VfsMeta,
    callback_path: &str,
    volume_serial: u32,
    read_only: bool,
) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    reject_open_symlink(meta)?;
    let size = meta.size;
    let index = stable_hash(meta.id.as_deref().unwrap_or(callback_path).as_bytes());
    Ok(BY_HANDLE_FILE_INFORMATION {
        dwFileAttributes: attributes(meta, read_only),
        ftCreationTime: file_time(if meta.btime_ms != 0 {
            meta.btime_ms
        } else {
            meta.mtime_ms
        }),
        ftLastAccessTime: file_time(meta.mtime_ms),
        ftLastWriteTime: file_time(meta.mtime_ms),
        dwVolumeSerialNumber: volume_serial,
        nFileSizeHigh: (size >> 32) as u32,
        nFileSizeLow: size as u32,
        nNumberOfLinks: 1,
        nFileIndexHigh: (index >> 32) as u32,
        nFileIndexLow: index as u32,
    })
}

pub(super) fn find_data(
    meta: &VfsMeta,
    display_name: &str,
    read_only: bool,
    special_directory_name: bool,
) -> io::Result<WIN32_FIND_DATAW> {
    let name = if special_directory_name {
        let mut value = [0u16; 260];
        let encoded = display_name.encode_utf16().collect::<Vec<_>>();
        value[..encoded.len()].copy_from_slice(&encoded);
        value
    } else {
        encode_find_name(display_name)?
    };
    let size = meta.size;
    Ok(WIN32_FIND_DATAW {
        dwFileAttributes: attributes(meta, read_only),
        ftCreationTime: file_time(if meta.btime_ms != 0 {
            meta.btime_ms
        } else {
            meta.mtime_ms
        }),
        ftLastAccessTime: file_time(meta.mtime_ms),
        ftLastWriteTime: file_time(meta.mtime_ms),
        nFileSizeHigh: (size >> 32) as u32,
        nFileSizeLow: size as u32,
        dwReserved0: 0,
        dwReserved1: 0,
        cFileName: name,
        cAlternateFileName: [0; 14],
    })
}

pub(super) fn reject_open_symlink(meta: &VfsMeta) -> io::Result<()> {
    if meta.is_symlink {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "remote links are visible but cannot be followed through the mounted drive",
        ));
    }
    Ok(())
}

pub(super) fn volume_serial(mount_id: &str) -> u32 {
    stable_hash(mount_id.as_bytes()) as u32
}

fn attributes(meta: &VfsMeta, read_only: bool) -> u32 {
    let mut value = if meta.is_dir {
        FILE_ATTRIBUTE_DIRECTORY
    } else {
        FILE_ATTRIBUTE_ARCHIVE
    };
    if meta.hidden {
        value |= FILE_ATTRIBUTE_HIDDEN;
    }
    if meta.system {
        value |= FILE_ATTRIBUTE_SYSTEM;
    }
    if read_only && !meta.is_dir {
        value |= FILE_ATTRIBUTE_READONLY;
    }
    if value == 0 {
        FILE_ATTRIBUTE_NORMAL
    } else {
        value
    }
}

fn file_time(unix_ms: i64) -> FILETIME {
    if unix_ms == 0 {
        return FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
    }
    let intervals =
        ((unix_ms as i128 + WINDOWS_EPOCH_OFFSET_MS) * 10_000).clamp(0, u64::MAX as i128) as u64;
    FILETIME {
        dwLowDateTime: intervals as u32,
        dwHighDateTime: (intervals >> 32) as u32,
    }
}

fn stable_hash(value: &[u8]) -> u64 {
    value.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x1000_0000_01b3)
    })
}
