//! Synchronous NT operations with a retained RootDirectory and one literal name.
use std::os::windows::{
    ffi::OsStrExt,
    io::{AsRawHandle, FromRawHandle},
};
use std::{
    ffi::{c_void, OsStr},
    fs::File,
    io,
    mem::{offset_of, size_of},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{RtlNtStatusToDosError, HANDLE, STATUS_PENDING, WAIT_OBJECT_0},
    System::{
        Threading::{WaitForSingleObject, INFINITE},
        IO::IO_STATUS_BLOCK,
    },
};

#[repr(C)]
struct Unicode {
    length: u16,
    maximum: u16,
    buffer: *mut u16,
}
#[repr(C)]
struct Attributes {
    length: u32,
    root: HANDLE,
    name: *mut Unicode,
    flags: u32,
    security: *mut c_void,
    qos: *mut c_void,
}
#[repr(C)]
struct Rename {
    flags: u32,
    root: HANDLE,
    length: u32,
    name: [u16; 1],
}

#[link(name = "ntdll")]
extern "system" {
    fn NtCreateFile(
        handle: *mut HANDLE,
        access: u32,
        attributes: *mut Attributes,
        completion: *mut IO_STATUS_BLOCK,
        allocation: *const i64,
        file_attributes: u32,
        sharing: u32,
        disposition: u32,
        options: u32,
        ea: *const c_void,
        ea_length: u32,
    ) -> i32;
    fn NtSetInformationFile(
        handle: HANDLE,
        completion: *mut IO_STATUS_BLOCK,
        information: *const c_void,
        length: u32,
        class: i32,
    ) -> i32;
}

pub(super) fn name(name: &OsStr) -> io::Result<Vec<u16>> {
    let wide: Vec<u16> = name.encode_wide().collect();
    if wide.is_empty()
        || name == OsStr::new(".")
        || name == OsStr::new("..")
        || wide.iter().any(|ch| matches!(*ch, 0 | 47 | 92 | 58))
        || wide.len() > (u16::MAX as usize / 2)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "not one literal Windows child name",
        ));
    }
    Ok(wide)
}

pub(super) fn open(
    parent: &File,
    child: &OsStr,
    access: u32,
    disposition: u32,
    options: u32,
    sharing: u32,
) -> io::Result<File> {
    let mut wide = name(child)?;
    let length = (wide.len() * 2) as u16;
    let mut name = Unicode {
        length,
        maximum: length,
        buffer: wide.as_mut_ptr(),
    };
    let mut attributes = Attributes {
        length: size_of::<Attributes>() as u32,
        root: parent.as_raw_handle(),
        name: &mut name,
        flags: 0x40,
        security: ptr::null_mut(),
        qos: ptr::null_mut(),
    };
    let mut completion: Box<IO_STATUS_BLOCK> = Box::new(unsafe { std::mem::zeroed() });
    let mut handle: HANDLE = ptr::null_mut();
    // FILE_SYNCHRONOUS_IO_NONALERT, FILE_OPEN_REPARSE_POINT; no Win32 name parsing.
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            access | 0x0010_0000,
            &mut attributes,
            completion.as_mut(),
            ptr::null(),
            0x80,
            sharing,
            disposition,
            options | 0x20 | 0x0020_0000,
            ptr::null(),
            0,
        )
    };
    if status < 0 {
        return Err(nt_error(status));
    }
    if handle.is_null() {
        return Err(io::Error::other("NT open returned no handle"));
    }
    let file = unsafe { File::from_raw_handle(handle) };
    if status == STATUS_PENDING {
        if unsafe { WaitForSingleObject(file.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
            // Preserve the live completion storage and handle on an unconfirmed I/O.
            std::mem::forget(completion);
            std::mem::forget(file);
            return Err(io::Error::other("relative open completion unconfirmed"));
        }
        let status = unsafe { completion.Anonymous.Status };
        if status < 0 {
            return Err(nt_error(status));
        }
        if status == STATUS_PENDING {
            std::mem::forget(completion);
            std::mem::forget(file);
            return Err(io::Error::other("relative open still pending"));
        }
    }
    Ok(file)
}

pub(super) fn rename(source: File, parent: &File, child: &OsStr, replace: bool) -> io::Result<()> {
    rename_with_class(
        source,
        parent,
        child,
        replace,
        if replace { 65 } else { 10 },
    )
}
pub(super) fn rename_basic(source: File, parent: &File, child: &OsStr) -> io::Result<()> {
    rename_with_class(source, parent, child, true, 10)
}
fn rename_with_class(
    source: File,
    parent: &File,
    child: &OsStr,
    replace: bool,
    class: i32,
) -> io::Result<()> {
    let wide = name(child)?;
    let target = parent.try_clone()?;
    let bytes = size_of::<Rename>()
        .checked_add(wide.len() * 2)
        .and_then(|length| u32::try_from(length).ok())
        .ok_or_else(|| io::Error::other("rename name too long"))?;
    let mut buffer = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
    let info = buffer.as_mut_ptr().cast::<Rename>();
    let mut completion: Box<IO_STATUS_BLOCK> = Box::new(unsafe { std::mem::zeroed() });
    // The Ex flags preserve replacing read-only destinations and existing readers.
    // FileRenameInformation (10) provides atomic no-replace on every supported OS.
    let status = unsafe {
        (*info).flags = if replace && class == 65 {
            0x1 | 0x2 | 0x40
        } else if replace {
            1
        } else {
            0
        };
        (*info).root = target.as_raw_handle();
        (*info).length = (wide.len() * 2) as u32;
        ptr::copy_nonoverlapping(
            wide.as_ptr(),
            buffer
                .as_mut_ptr()
                .cast::<u8>()
                .add(offset_of!(Rename, name))
                .cast::<u16>(),
            wide.len(),
        );
        NtSetInformationFile(
            source.as_raw_handle(),
            completion.as_mut(),
            info.cast(),
            bytes,
            class,
        )
    };
    let status = if status == STATUS_PENDING {
        if unsafe { WaitForSingleObject(source.as_raw_handle(), INFINITE) } != WAIT_OBJECT_0 {
            std::mem::forget((source, target, completion, buffer));
            return Err(io::Error::other("relative rename completion unconfirmed"));
        }
        let result = unsafe { completion.Anonymous.Status };
        if result == STATUS_PENDING {
            std::mem::forget((source, target, completion, buffer));
            return Err(io::Error::other("relative rename still pending"));
        }
        result
    } else {
        status
    };
    if status < 0 {
        return Err(nt_error(status));
    }
    Ok(())
}
fn nt_error(status: i32) -> io::Error {
    io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32)
}
