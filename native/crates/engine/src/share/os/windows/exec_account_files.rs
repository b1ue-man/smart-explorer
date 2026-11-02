//! Files of the Windows account service in `%ProgramData%\Smart Explorer`
//! (D.2.8, C5) and the SHA-256 of program files. Users may create folders
//! in `%ProgramData%`, so a file is trusted only when neither it nor its
//! folder is a link, the file has exactly one name (no hard link) and both
//! belong to SYSTEM or the Administrators group. The elevated setup creates
//! the folder for SYSTEM and the Administrators (users read it) and refuses
//! a folder that belongs to anyone else. A file is written beside its
//! target with its own access list and renamed over it.
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::windows::io::FromRawHandle;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use sha2::{Digest, Sha256};
use windows_sys::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, GENERIC_WRITE, HANDLE,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{SetSecurityInfo, SE_FILE_OBJECT};
use windows_sys::Win32::Security::{
    WinBuiltinAdministratorsSid, WinLocalSystemSid, DACL_SECURITY_INFORMATION,
    PROTECTED_DACL_SECURITY_INFORMATION, SECURITY_ATTRIBUTES,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateDirectoryW, CreateFileW, GetFileInformationByHandle, MoveFileExW,
    BY_HANDLE_FILE_INFORMATION, CREATE_NEW, FILE_APPEND_DATA, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    OPEN_ALWAYS, OPEN_EXISTING, READ_CONTROL, WRITE_DAC,
};

use crate::share::exec_types::ExecId;

use super::exec_account_config::{data_dir, log_path};
use super::exec_account_security::{owner_of, wide, SecurityDescriptor, Sid};
use super::exec_handles::OwnedHandle;

/// The data folder: SYSTEM and the Administrators change it, users read it.
const DATA_DIR_SDDL: &str = "O:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)";
/// The configuration (D.2.8): users read it.
pub(super) const CONFIG_SDDL: &str = "O:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FR;;;BU)";
/// The service log (C5): SYSTEM and the Administrators only.
const LOG_SDDL: &str = "O:SYD:P(A;;FA;;;SY)(A;;FA;;;BA)";
/// A log larger than this moves to `exec-accounts.log.1` before the next line.
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;

/// The report of an elevated setup: the user `sid` may read it.
pub(super) fn report_sddl(sid: &str) -> String {
    format!("O:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FR;;;{sid})")
}

fn open(
    path: &Path,
    access: u32,
    disposition: u32,
    flags: u32,
    attributes: Option<&SECURITY_ATTRIBUTES>,
) -> io::Result<OwnedHandle> {
    let name = wide(path);
    let share = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
    let attributes = attributes.map_or(null(), |attributes| attributes as *const _);
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            access,
            share,
            attributes,
            disposition,
            flags,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    OwnedHandle::new(handle)
}

fn information(handle: HANDLE) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(handle, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}

fn is_link(info: &BY_HANDLE_FILE_INFORMATION) -> bool {
    info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn is_directory(info: &BY_HANDLE_FILE_INFORMATION) -> bool {
    info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0
}

/// The object behind `handle` belongs to SYSTEM or the Administrators.
fn trusted_owner(path: &Path, handle: HANDLE) -> io::Result<()> {
    let owner = owner_of(handle, SE_FILE_OBJECT)?;
    if owner == Sid::well_known(WinLocalSystemSid)?
        || owner == Sid::well_known(WinBuiltinAdministratorsSid)?
    {
        Ok(())
    } else {
        Err(untrusted(
            path,
            "belongs to someone else than SYSTEM or the Administrators",
        ))
    }
}

pub(super) fn is_not_found(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error().map(|code| code as u32),
        Some(ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND)
    )
}

/// `directory` is a folder, no link, of SYSTEM or the Administrators.
pub(super) fn trusted_dir(directory: &Path) -> io::Result<()> {
    let flags = FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT;
    let access = READ_CONTROL | FILE_READ_ATTRIBUTES;
    let handle = open(directory, access, OPEN_EXISTING, flags, None)?;
    let info = information(handle.raw())?;
    if is_link(&info) || !is_directory(&info) {
        return Err(untrusted(directory, "is a link or no folder"));
    }
    trusted_owner(directory, handle.raw())
}

/// The bytes (at most `limit`) of the trusted file `path`; `Ok(None)` when
/// it or its folder is missing.
pub(super) fn read_trusted(path: &Path, limit: u64) -> io::Result<Option<Vec<u8>>> {
    let directory = path
        .parent()
        .ok_or_else(|| untrusted(path, "has no folder"))?;
    match trusted_dir(directory) {
        Err(error) if is_not_found(&error) => return Ok(None),
        Err(error) => return Err(error),
        Ok(()) => {}
    }
    let opened = open(
        path,
        FILE_GENERIC_READ,
        OPEN_EXISTING,
        FILE_FLAG_OPEN_REPARSE_POINT,
        None,
    );
    let handle = match opened {
        Err(error) if is_not_found(&error) => return Ok(None),
        other => other?,
    };
    let info = information(handle.raw())?;
    if is_link(&info) || is_directory(&info) || info.nNumberOfLinks != 1 {
        return Err(untrusted(
            path,
            "is a link, a folder or has more than one name",
        ));
    }
    trusted_owner(path, handle.raw())?;
    // SAFETY: the handle is open and owned by nothing else from here on.
    let file = unsafe { File::from_raw_handle(handle.into_raw().cast()) };
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(untrusted(path, "is larger than allowed"));
    }
    Ok(Some(bytes))
}

/// Creates `%ProgramData%\Smart Explorer` for SYSTEM and the Administrators
/// (users read it), or gives an existing folder of theirs that access list.
/// A link, or a folder of anyone else (it could hold planted files), is
/// refused. Needs administrator rights.
pub(super) fn ensure_data_dir() -> io::Result<PathBuf> {
    let directory = data_dir()?;
    let descriptor = SecurityDescriptor::from_sddl(DATA_DIR_SDDL)?;
    let attributes = descriptor.attributes(false);
    let name = wide(&directory);
    if unsafe { CreateDirectoryW(name.as_ptr(), &attributes) } != 0 {
        return Ok(directory);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) {
        return Err(error);
    }
    let flags = FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT;
    let access = READ_CONTROL | WRITE_DAC | FILE_READ_ATTRIBUTES;
    let handle = open(&directory, access, OPEN_EXISTING, flags, None)?;
    let info = information(handle.raw())?;
    if is_link(&info) || !is_directory(&info) {
        return Err(untrusted(
            &directory,
            "is a link or no folder; an administrator removes it",
        ));
    }
    trusted_owner(&directory, handle.raw()).map_err(|error| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{error}; an administrator removes the folder, then sets up again"),
        )
    })?;
    let status = unsafe {
        SetSecurityInfo(
            handle.raw(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            descriptor.dacl()?,
            null(),
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    Ok(directory)
}

/// Writes `bytes` as the file `path` of the trusted folder with the access
/// list `sddl`: a new file beside it, written through, renamed over it.
pub(super) fn write_trusted(path: &Path, bytes: &[u8], sddl: &str) -> io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| untrusted(path, "has no folder"))?;
    trusted_dir(directory)?;
    let descriptor = SecurityDescriptor::from_sddl(sddl)?;
    let attributes = descriptor.attributes(false);
    let temporary = path.with_extension(format!("tmp-{}", ExecId::generate()?.as_str()));
    let handle = open(
        &temporary,
        GENERIC_WRITE,
        CREATE_NEW,
        FILE_ATTRIBUTE_NORMAL,
        Some(&attributes),
    )?;
    // SAFETY: the handle is open and owned by nothing else from here on.
    let mut file = unsafe { File::from_raw_handle(handle.into_raw().cast()) };
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let moved = written.and_then(|()| {
        let (from, to) = (wide(&temporary), wide(path));
        let flags = MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH;
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), flags) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    });
    if moved.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    moved
}

/// Removes `path`; `Ok(false)` when there was none.
pub(super) fn remove_file(path: &Path) -> io::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Appends one line with the time to the service log (C5). Best effort: a
/// missing or untrusted folder or a full disk never fails a request.
pub(super) fn append_log(line: &str) {
    let _ = try_append_log(line);
}

fn try_append_log(line: &str) -> io::Result<()> {
    let path = log_path()?;
    let directory = path
        .parent()
        .ok_or_else(|| untrusted(&path, "has no folder"))?;
    trusted_dir(directory)?;
    if std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.len() > MAX_LOG_BYTES) {
        let (from, to) = (wide(&path), wide(path.with_extension("log.1")));
        unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_REPLACE_EXISTING) };
    }
    let descriptor = SecurityDescriptor::from_sddl(LOG_SDDL)?;
    let attributes = descriptor.attributes(false);
    let access = FILE_APPEND_DATA | READ_CONTROL | FILE_READ_ATTRIBUTES;
    let flags = FILE_FLAG_OPEN_REPARSE_POINT;
    let handle = open(&path, access, OPEN_ALWAYS, flags, Some(&attributes))?;
    let info = information(handle.raw())?;
    if is_link(&info) || is_directory(&info) || info.nNumberOfLinks != 1 {
        return Err(untrusted(
            &path,
            "is a link, a folder or has more than one name",
        ));
    }
    trusted_owner(&path, handle.raw())?;
    // SAFETY: the handle is open and owned by nothing else from here on.
    let mut file = unsafe { File::from_raw_handle(handle.into_raw().cast()) };
    let time = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let text = line.replace(['\r', '\n'], " ");
    file.write_all(format!("{time} {text}\n").as_bytes())
}

/// SHA-256 (lowercase hex) of the file `path`.
pub(super) fn sha256_file(path: &Path) -> io::Result<String> {
    sha256_reader(&mut File::open(path)?)
}

/// SHA-256 (lowercase hex) of everything `reader` yields.
pub(super) fn sha256_reader(reader: &mut impl Read) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn untrusted(path: &Path, why: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{} {why}", path.display()),
    )
}
