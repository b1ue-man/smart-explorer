//! Held-object publication opens and actual directory durability confirmation.
use std::{
    ffi::{c_void, OsStr},
    fs::File,
    io,
    os::windows::io::{AsRawHandle, FromRawHandle},
};
use windows_sys::Win32::{
    Foundation::{
        RtlNtStatusToDosError, HANDLE, INVALID_HANDLE_VALUE, STATUS_PENDING, WAIT_OBJECT_0,
    },
    Storage::FileSystem::{
        GetVolumeInformationByHandleW, FILE_ADD_FILE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    },
    System::{
        Threading::{WaitForSingleObject, INFINITE},
        IO::IO_STATUS_BLOCK,
    },
};

#[link(name = "ntdll")]
extern "system" {
    fn NtCreateFile(
        handle: *mut HANDLE,
        access: u32,
        attributes: *mut ObjectAttributes,
        completion: *mut IO_STATUS_BLOCK,
        allocation: *const i64,
        file_attributes: u32,
        sharing: u32,
        disposition: u32,
        options: u32,
        ea: *const c_void,
        ea_length: u32,
    ) -> i32;
    fn NtQueryVolumeInformationFile(
        file: HANDLE,
        completion: *mut IO_STATUS_BLOCK,
        data: *mut c_void,
        bytes: u32,
        class: i32,
    ) -> i32;
    fn NtFlushBuffersFileEx(
        file: HANDLE,
        flags: u32,
        parameters: *const c_void,
        parameter_bytes: u32,
        completion: *mut IO_STATUS_BLOCK,
    ) -> i32;
}

#[repr(C)]
struct Unicode {
    length: u16,
    maximum: u16,
    buffer: *mut u16,
}

#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root: HANDLE,
    name: *mut Unicode,
    flags: u32,
    security: *mut c_void,
    qos: *mut c_void,
}

// All native output and name storage stays at stable addresses if an unusual
// pending open cannot be confirmed. The retained parent is never a path reopen.
struct DirectoryOpen {
    handle: HANDLE,
    completion: IO_STATUS_BLOCK,
    name: Unicode,
    attributes: ObjectAttributes,
    empty: [u16; 1],
    _parent: File,
}

pub(super) fn open_file(parent: &File, name: &OsStr) -> io::Result<File> {
    // FILE_OPEN, NON_DIRECTORY_FILE, WRITE_THROUGH. Refusing write/delete
    // sharing prevents concurrent writers/renames during the publication proof.
    // In contrast to ordinary writes this never clears the read-only bit and
    // does not reject an already published hardlink merely for its link count.
    let file = super::native::open(
        parent,
        name,
        FILE_GENERIC_READ | FILE_GENERIC_WRITE,
        1,
        0x40 | 0x2,
        FILE_SHARE_READ,
    )?;
    super::regular::validate_file(&file, true)?;
    Ok(file)
}

pub(super) fn open_move_file(parent: &File, name: &OsStr) -> io::Result<File> {
    // A move must retain this flush-capable handle across its own rename.
    // Readonly/access/sharing failure selects the established staged path.
    let file = super::native::open(
        parent,
        name,
        FILE_GENERIC_READ | FILE_GENERIC_WRITE,
        1,
        0x40 | 0x2,
        FILE_SHARE_READ | FILE_SHARE_DELETE,
    )?;
    super::regular::validate_file(&file, true)?;
    Ok(file)
}

pub(super) fn confirm_namespace(parent: &File) -> io::Result<()> {
    validate_directory(parent)?;
    require_namespace_support(parent)?;
    let file = reopen_directory(parent)?;
    flush_directory(file)
}

fn validate_directory(file: &File) -> io::Result<()> {
    let class = super::super::directory::classify_open_file(file)?;
    if !file.metadata()?.is_dir() || class.link_like || class.special {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "namespace flush requires the held physical directory",
        ));
    }
    Ok(())
}

fn reopen_directory(parent: &File) -> io::Result<File> {
    validate_directory(parent)?;
    let parent = parent.try_clone()?;
    let mut request = Box::new(DirectoryOpen {
        handle: std::ptr::null_mut(),
        completion: unsafe { std::mem::zeroed() },
        name: Unicode {
            length: 0,
            maximum: 2,
            buffer: std::ptr::null_mut(),
        },
        attributes: ObjectAttributes {
            length: std::mem::size_of::<ObjectAttributes>() as u32,
            root: parent.as_raw_handle(),
            name: std::ptr::null_mut(),
            flags: 0,
            security: std::ptr::null_mut(),
            qos: std::ptr::null_mut(),
        },
        empty: [0],
        _parent: parent,
    });
    let raw = &mut *request;
    raw.name.buffer = raw.empty.as_mut_ptr();
    raw.attributes.name = &mut raw.name;
    // ReOpenFile rejects directories, including with BACKUP_SEMANTICS.
    // An empty native name reopens this object (Microsoft WSL's ReopenFile
    // uses the same boundary). FILE_OPEN cannot create or replace anything.
    // ADD_FILE is the directory spelling of the write bit required by flush;
    // no EA, DACL, attribute-write or add-subdirectory grant is requested.
    let status = unsafe {
        NtCreateFile(
            &mut raw.handle,
            FILE_ADD_FILE | FILE_READ_ATTRIBUTES | 0x0010_0000,
            &mut raw.attributes,
            &mut raw.completion,
            std::ptr::null(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            1,          // FILE_OPEN
            0x1 | 0x20, // DIRECTORY_FILE | SYNCHRONOUS_IO_NONALERT
            std::ptr::null(),
            0,
        )
    };
    let Some(status) = completed_status(raw.handle, status, &raw.completion) else {
        std::mem::forget(request);
        return Err(unconfirmed("directory reopen"));
    };
    require_success(status)?;
    if raw.handle.is_null() || raw.handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::other("directory reopen returned no handle"));
    }
    let file = unsafe { File::from_raw_handle(raw.handle) };
    validate_directory(&file)?;
    if raw.completion.Information != 1 || super::identity(&file)? != super::identity(&raw._parent)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "directory reopen changed its held object",
        ));
    }
    Ok(file)
}

fn require_namespace_support(parent: &File) -> io::Result<()> {
    #[cfg(test)]
    task_tests::namespace_support()?;
    let file = parent.try_clone()?;
    let mut device = Box::new([0u32; 2]);
    let mut completion: Box<IO_STATUS_BLOCK> = Box::new(unsafe { std::mem::zeroed() });
    let status = unsafe {
        NtQueryVolumeInformationFile(
            file.as_raw_handle(),
            completion.as_mut(),
            device.as_mut_ptr().cast(),
            8,
            4,
        )
    };
    let Some(status) = completed_status(file.as_raw_handle(), status, &completion) else {
        std::mem::forget((file, device, completion));
        return Err(unconfirmed("directory volume query"));
    };
    require_success(status)?;
    if completion.Information != 8 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "incomplete directory volume query",
        ));
    }
    let mut name = [0u16; 261];
    if unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            name.as_mut_ptr(),
            name.len() as u32,
        )
    } == 0 {
        return Err(io::Error::last_os_error());
    }
    let length = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(name.len());
    if !namespace_supported(&String::from_utf16_lossy(&name[..length]), device[1]) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "provider has no confirmed directory-structure persistence contract",
        ));
    }
    Ok(())
}

fn namespace_supported(filesystem: &str, characteristics: u32) -> bool {
    // MS-FSA 2.1.5.7 / product note 80: non-NTFS directory flushes may
    // succeed without persisting structure. A remote NTFS label is not a
    // local filesystem proof. This never restricts live write-through moves.
    filesystem.eq_ignore_ascii_case("NTFS") && characteristics & (0x10 | 0x2000) == 0
}

fn flush_directory(file: File) -> io::Result<()> {
    #[cfg(test)]
    task_tests::before_flush(&file)?;
    let mut completion: Box<IO_STATUS_BLOCK> = Box::new(unsafe { std::mem::zeroed() });
    // Normal flags (0) flush data, metadata and underlying storage cache.
    // No DATA_ONLY/NO_SYNC shortcut can confirm a published directory entry.
    let status = unsafe {
        NtFlushBuffersFileEx(
            file.as_raw_handle(),
            0,
            std::ptr::null(),
            0,
            completion.as_mut(),
        )
    };
    let Some(status) = completed_status(file.as_raw_handle(), status, &completion) else {
        std::mem::forget((file, completion));
        return Err(unconfirmed("publication namespace flush"));
    };
    require_success(status)
}

fn completed_status(file: HANDLE, status: i32, completion: &IO_STATUS_BLOCK) -> Option<i32> {
    if status != STATUS_PENDING {
        return Some(status);
    }
    if file.is_null()
        || file == INVALID_HANDLE_VALUE
        || unsafe { WaitForSingleObject(file, INFINITE) } != WAIT_OBJECT_0
    {
        return None;
    }
    let status = unsafe { completion.Anonymous.Status };
    (status != STATUS_PENDING).then_some(status)
}

fn require_success(status: i32) -> io::Result<()> {
    if status < 0 {
        Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(status) } as i32,
        ))
    } else if status != 0 {
        Err(io::Error::other(
            "publication directory operation did not confirm success",
        ))
    } else {
        Ok(())
    }
}

fn unconfirmed(operation: &str) -> io::Error {
    io::Error::other(format!("{operation} completion unconfirmed"))
}

#[cfg(test)]
#[path = "publication_task_tests.rs"]
mod task_tests;
