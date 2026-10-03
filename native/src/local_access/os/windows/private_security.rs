//! Protected DACLs containing only the effective token user. All security
//! probes use opened handles; unsupported providers get no record contents.
use std::fs::File;
use std::io;
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;

use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_NO_TOKEN, GENERIC_READ, GENERIC_WRITE, HANDLE,
        INVALID_HANDLE_VALUE,
    },
    Security::{
        AddAccessAllowedAce,
        Authorization::{SetSecurityInfo, SE_FILE_OBJECT},
        EqualSid, GetAce, GetKernelObjectSecurity, GetLengthSid, GetSecurityDescriptorControl,
        GetSecurityDescriptorDacl, GetSecurityDescriptorOwner, GetTokenInformation, InitializeAcl,
        InitializeSecurityDescriptor, SetSecurityDescriptorControl, SetSecurityDescriptorDacl,
        SetSecurityDescriptorOwner, TokenUser, ACCESS_ALLOWED_ACE, ACL, DACL_SECURITY_INFORMATION,
        OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSID, SECURITY_ATTRIBUTES,
        SECURITY_DESCRIPTOR, SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
    },
    Storage::FileSystem::{
        CreateDirectoryW, CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        CREATE_NEW, FILE_ALL_ACCESS, FILE_ATTRIBUTE_DEVICE, FILE_ATTRIBUTE_NORMAL,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, READ_CONTROL,
    },
    System::Threading::{GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken},
};

const ACL_REVISION: u32 = 2;
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;

struct Token(HANDLE);

impl Drop for Token {
    fn drop(&mut self) {
        // SAFETY: owns one successfully opened token handle.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct PrivateSecurity {
    descriptor: SECURITY_DESCRIPTOR,
    _acl: Vec<u64>,
    user: Vec<u64>,
}

impl PrivateSecurity {
    fn new() -> io::Result<Self> {
        let user = token_user()?;
        // SAFETY: token_user returned a complete, aligned TOKEN_USER buffer.
        let sid = unsafe { (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid };
        let length = unsafe { GetLengthSid(sid) } as usize;
        let acl_bytes = size_of::<ACL>() + offset_of!(ACCESS_ALLOWED_ACE, SidStart) + length;
        let mut acl = vec![0u64; acl_bytes.div_ceil(size_of::<u64>())];
        let acl_pointer = acl.as_mut_ptr().cast::<ACL>();
        // SAFETY: all buffers and SID remain live; Windows initializes the
        // structures and copies the SID into the appropriately sized ACL.
        let mut descriptor: SECURITY_DESCRIPTOR = unsafe { std::mem::zeroed() };
        let pointer = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
        unsafe {
            win(InitializeAcl(acl_pointer, acl_bytes as u32, ACL_REVISION))?;
            win(AddAccessAllowedAce(
                acl_pointer,
                ACL_REVISION,
                FILE_ALL_ACCESS,
                sid,
            ))?;
            win(InitializeSecurityDescriptor(
                pointer,
                SECURITY_DESCRIPTOR_REVISION,
            ))?;
            win(SetSecurityDescriptorOwner(pointer, sid, 0))?;
            win(SetSecurityDescriptorDacl(pointer, 1, acl_pointer, 0))?;
            win(SetSecurityDescriptorControl(
                pointer,
                SE_DACL_PROTECTED,
                SE_DACL_PROTECTED,
            ))?;
        }
        Ok(Self {
            descriptor,
            _acl: acl,
            user,
        })
    }

    fn attributes(&mut self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: (&mut self.descriptor as *mut SECURITY_DESCRIPTOR).cast(),
            bInheritHandle: 0,
        }
    }

    fn sid(&self) -> PSID {
        // SAFETY: owns the still-live complete TOKEN_USER buffer.
        unsafe { (*(self.user.as_ptr().cast::<TOKEN_USER>())).User.Sid }
    }
}

pub(super) fn create_directory(path: &Path) -> io::Result<File> {
    let mut security = PrivateSecurity::new()?;
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let attributes = security.attributes();
    // SAFETY: both NUL-terminated path and absolute descriptor remain live.
    win(unsafe { CreateDirectoryW(name.as_ptr(), &attributes) })?;
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY | READ_CONTROL)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    validate_object(&file, true)?;
    verify_private(&file, security.sid())?;
    Ok(file)
}

pub(super) fn create_file(path: &Path) -> io::Result<File> {
    let mut security = PrivateSecurity::new()?;
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let attributes = security.attributes();
    // SAFETY: path, attributes and descriptor are live; CREATE_NEW is
    // exclusive and never follows an existing final reparse point.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE | READ_CONTROL | FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: owns this new handle exactly once and closes on every failure.
    let file = unsafe { File::from_raw_handle(handle) };
    super::super::super::regular::validate_file(&file, true)?;
    validate_object(&file, false)?;
    verify_private(&file, security.sid())?;
    Ok(file)
}

/// Validate ownership and kind before tightening only this opened object.
/// No owner takeover, named-path ACL mutation or privilege activation.
pub(crate) fn secure_private_handle(file: &File, is_directory: bool) -> io::Result<()> {
    validate_object(file, is_directory)?;
    let mut security = PrivateSecurity::new()?;
    let mut owned = descriptor(file, OWNER_SECURITY_INFORMATION)?;
    let mut owner = std::ptr::null_mut();
    let mut defaulted = 0;
    win(unsafe {
        GetSecurityDescriptorOwner(owned.as_mut_ptr().cast(), &mut owner, &mut defaulted)
    })?;
    if owner.is_null() || unsafe { EqualSid(owner, security.sid()) } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private object has a foreign owner",
        ));
    }
    match verify_private(file, security.sid()) {
        Ok(()) => return validate_object(file, is_directory),
        Err(error)
            if error.kind() == io::ErrorKind::PermissionDenied
                && error.raw_os_error().is_none() => {}
        Err(error) => return Err(error),
    }
    // This non-inheritable owner ACE introduces no grants to descendants.
    // SetSecurityInfo is the documented handle API for filesystem objects;
    // only the DACL changes, never the owner or a SACL.
    let error = unsafe {
        SetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            security._acl.as_mut_ptr().cast(),
            std::ptr::null_mut(),
        )
    };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    verify_private(file, security.sid())?;
    validate_object(file, is_directory)
}

fn validate_object(file: &File, is_directory: bool) -> io::Result<()> {
    let metadata = file.metadata()?;
    if metadata.file_attributes() & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DEVICE) != 0
        || (is_directory && !metadata.is_dir())
        || (!is_directory && !metadata.is_file())
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "private object is redirected or special",
        ));
    }
    if !is_directory {
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        win(unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) })?;
        if info.nNumberOfLinks != 1 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "private file has another hardlink",
            ));
        }
    }
    Ok(())
}

fn token_user() -> io::Result<Vec<u64>> {
    let mut handle = std::ptr::null_mut();
    // SAFETY: plain token query; no privileges or impersonation are changed.
    if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut handle) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_NO_TOKEN as i32) {
            return Err(error);
        }
        win(unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) })?;
    }
    let token = Token(handle);
    let mut needed = 0;
    let first =
        unsafe { GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut needed) };
    if first != 0
        || io::Error::last_os_error().raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32)
    {
        return Err(io::Error::last_os_error());
    }
    if needed < size_of::<TOKEN_USER>() as u32 || needed > 64 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid token user size",
        ));
    }
    let mut buffer = vec![0u64; (needed as usize).div_ceil(size_of::<u64>())];
    win(unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    })?;
    Ok(buffer)
}

fn descriptor(file: &File, information: u32) -> io::Result<Vec<u64>> {
    let mut needed = 0;
    // SAFETY: a live metadata-capable file; first query sizes the buffer.
    let first = unsafe {
        GetKernelObjectSecurity(
            file.as_raw_handle(),
            information,
            std::ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    if first != 0
        || io::Error::last_os_error().raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32)
    {
        return Err(io::Error::last_os_error());
    }
    // GetKernelObjectSecurity returns the 20-byte self-relative header, not
    // the pointer-sized absolute SECURITY_DESCRIPTOR layout.
    if needed < 20 || needed > 64 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid private descriptor size",
        ));
    }
    let mut buffer = vec![0u64; (needed as usize).div_ceil(size_of::<u64>())];
    let descriptor = buffer.as_mut_ptr().cast();
    win(unsafe {
        GetKernelObjectSecurity(
            file.as_raw_handle(),
            information,
            descriptor,
            needed,
            &mut needed,
        )
    })?;
    Ok(buffer)
}

fn verify_private(file: &File, expected_owner: PSID) -> io::Result<()> {
    let mut buffer = descriptor(file, DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION)?;
    let descriptor = buffer.as_mut_ptr().cast();
    let mut owner = std::ptr::null_mut();
    let mut defaulted = 0;
    let mut present = 0;
    let mut acl = std::ptr::null_mut();
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: the OS returned a complete security descriptor in live storage.
    unsafe {
        win(GetSecurityDescriptorOwner(
            descriptor,
            &mut owner,
            &mut defaulted,
        ))?;
        win(GetSecurityDescriptorDacl(
            descriptor,
            &mut present,
            &mut acl,
            &mut defaulted,
        ))?;
        win(GetSecurityDescriptorControl(
            descriptor,
            &mut control,
            &mut revision,
        ))?;
        if owner.is_null()
            || EqualSid(owner, expected_owner) == 0
            || present == 0
            || acl.is_null()
            || control & SE_DACL_PROTECTED == 0
            || (*acl).AceCount != 1
        {
            return Err(not_private());
        }
        let mut ace = std::ptr::null_mut();
        win(GetAce(acl, 0, &mut ace))?;
        let ace = ace.cast::<ACCESS_ALLOWED_ACE>();
        if (*ace).Header.AceType != 0
            || (*ace).Header.AceFlags != 0
            || (*ace).Mask != FILE_ALL_ACCESS
            || EqualSid(
                std::ptr::addr_of_mut!((*ace).SidStart).cast(),
                expected_owner,
            ) == 0
        {
            return Err(not_private());
        }
    }
    Ok(())
}

fn win(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn not_private() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "provider did not retain an owner-private protected DACL",
    )
}
