use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

pub type FileIdentity = (u32, u64);
#[path = "../../../../../core/src/host/os/windows_file_proof.rs"]
mod persistent_file_proof;
pub use persistent_file_proof::{same_file_object, stable_handle_identity};

pub fn metadata_is_link_like(path: &Path, metadata: &std::fs::Metadata) -> bool {
    metadata_class(path, metadata).0
}

pub fn metadata_class(path: &Path, metadata: &std::fs::Metadata) -> (bool, bool) {
    use std::os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    };
    use windows_sys::Win32::Storage::FileSystem::*;

    if metadata.is_symlink() {
        return (true, false);
    }
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
        return (
            false,
            metadata.file_attributes() & FILE_ATTRIBUTE_DEVICE != 0,
        );
    }
    let Ok(file) = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
    else {
        return (true, false);
    };
    let mut info: FILE_ATTRIBUTE_TAG_INFO = unsafe { std::mem::zeroed() };
    let read = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    };
    classify_tag(read != 0, info.ReparseTag, metadata.file_attributes())
}

/// Whether `metadata` carries the hidden attribute (`FILE_ATTRIBUTE_HIDDEN`),
/// the rule the Share host lists by on Windows (`vfs::local_attrs`); the name
/// does not matter. Used by the walk of a local sync side
/// (`walk_keyed::local_hidden`).
pub fn hidden_attribute(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN;
    metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
}

fn classify_tag(known: bool, tag: u32, attributes: u32) -> (bool, bool) {
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DEVICE;
    let link = !known || tag == 0 || tag & 0x2000_0000 != 0;
    // AF_UNIX / LX_FIFO / LX_CHR / LX_BLK (WDK tags); cloud, WOF and
    // deduplication data reparse points remain ordinary files/directories.
    let special = !link
        && (attributes & FILE_ATTRIBUTE_DEVICE != 0 || matches!(tag, 0x8000_0023..=0x8000_0026));
    (link, special)
}

pub fn open_regular_no_follow(path: &Path, write: bool) -> io::Result<std::fs::File> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::*;
    let file = std::fs::OpenOptions::new()
        .read(!write)
        .write(write)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = file.metadata()?;
    use std::os::windows::fs::MetadataExt;
    let mut info: FILE_ATTRIBUTE_TAG_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: the handle and output buffer remain live across the query.
    let read = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    };
    let (link, special) = if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        classify_tag(read != 0, info.ReparseTag, metadata.file_attributes())
    } else {
        (
            false,
            metadata.file_attributes() & FILE_ATTRIBUTE_DEVICE != 0,
        )
    };
    if link || special || !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "keine reguläre Datei",
        ));
    }
    Ok(file)
}

pub fn set_file_mode(_file: &std::fs::File, _mode: u32) -> io::Result<()> {
    Ok(())
}

/// Opens `path` for reading only when it is a regular file.
pub fn open_regular_file(path: &Path) -> io::Result<std::fs::File> {
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("keine reguläre Datei: {}", path.display()),
        ));
    }
    Ok(file)
}

/// Required stages (including Deferred) are already flushed individually;
/// both publication operations use write-through renames. Confirm only
/// that existing protocol contract, without claiming a volume flush.
pub fn sync_filesystem(path: &Path) -> io::Result<bool> {
    if !std::fs::metadata(path)?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "namespace parent must be a directory",
        ));
    }
    Ok(true)
}

pub fn file_identity(file: &std::fs::File) -> io::Result<FileIdentity> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut information) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Ok((information.dwVolumeSerialNumber, index))
}

pub fn path_matches_identity(path: &Path, expected: FileIdentity) -> io::Result<bool> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    Ok(file_identity(&file)? == expected)
}

/// Writes all of `buf` at `offset` (`seek_write` in a loop; it moves the file
/// pointer, which a stage that is only written at offsets never reads).
pub fn write_all_at(
    file: &std::fs::File,
    mut buf: &[u8],
    mut offset: u64,
) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match file.seek_write(buf, offset) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "failed to write whole buffer",
                ))
            }
            Ok(written) => {
                buf = &buf[written..];
                offset += written as u64;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Bytes this process may still use on the volume holding `dir`.
pub fn available_space(dir: &Path) -> io::Result<u64> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut available = 0u64;
    // SAFETY: `wide` is NUL-terminated and outlives the call; the two unused
    // out parameters are optional (null).
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(available)
}

/// Tag of the current user for names in a temp folder (which is per user).
pub fn user_tag() -> String {
    "user".to_string()
}

/// Creates `dir` for private state; a folder under the user's temp or data
/// folder inherits an ACL that excludes other users.
pub fn prepare_private_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)
}

/// Key of the volume holding `dir` (its drive or UNC share, case-folded), so
/// stages on one disk share one budget.
pub fn space_key(dir: &Path) -> io::Result<String> {
    use std::path::Component;
    match dir.components().next() {
        Some(Component::Prefix(prefix)) => Ok(prefix.as_os_str().to_string_lossy().to_lowercase()),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "path has no volume",
        )),
    }
}

pub fn secure_staging_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

pub fn secure_staging_file(_file: &std::fs::File) -> io::Result<()> {
    Ok(())
}

pub fn replace_file_atomic(source: &Path, destination: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let ok = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Atomically move `source` to a name that must not already exist.
pub fn rename_no_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // Omitting MOVEFILE_REPLACE_EXISTING is the Win32 no-replace primitive.
    let ok = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[path = "file_ids.rs"]
mod file_ids;
#[path = "folder_reservation.rs"]
pub mod folder_reservation;

/// The status of the open file the digest cache keys on (`hash_cache.rs`):
/// volume serial, 128-bit file ID (`FileIdInfo`), size, last write and
/// change time (`FileBasicInfo`, `FILE_BASIC_INFO.ChangeTime`) in Unix
/// nanoseconds. Every write moves the change time, as does a rename or an
/// attribute change. A program that holds the file with `FILE_WRITE_ATTRIBUTES`
/// can set it, though: `FILE_BASIC_INFO` is also the input of
/// `SetFileInformationByHandle(FileBasicInfo)`, which takes a `ChangeTime`
/// (assumption: NTFS keeps the value given). So the change time shows every
/// ordinary change and proves nothing against a program that sets it back on
/// purpose; the same is true of the other stamps of every file system. A
/// volume that is not NTFS or ReFS, and a network volume, has no stamp
/// (`file_ids.rs`).
pub fn file_stamp(file: &std::fs::File) -> Option<[i128; 5]> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileBasicInfo, FileIdInfo, GetFileInformationByHandleEx, FILE_BASIC_INFO, FILE_ID_INFO,
    };
    if !file_ids::file_has_durable_ids(file) {
        return None;
    }
    let mut basic: FILE_BASIC_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: the handle is open for the call; the buffer has the size given.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
    };
    if ok == 0 {
        return None;
    }
    let mut id: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: as above.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut id as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    let index = u128::from_le_bytes(id.FileId.Identifier);
    if ok == 0 || index == 0 || index == u128::MAX {
        return None;
    }
    let size = file.metadata().ok()?.len();
    Some([
        i128::from(id.VolumeSerialNumber),
        index as i128,
        i128::from(size),
        unix_ns(basic.LastWriteTime),
        unix_ns(basic.ChangeTime),
    ])
}

/// The status of `path` itself: opened for attributes only, a link as the
/// link (`FILE_FLAG_OPEN_REPARSE_POINT`).
pub fn path_stamp(path: &Path) -> Option<[i128; 5]> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    file_stamp(&file)
}

/// 100-ns ticks since 1601 as nanoseconds since 1970.
fn unix_ns(ticks: i64) -> i128 {
    (i128::from(ticks) - 116_444_736_000_000_000) * 100
}
