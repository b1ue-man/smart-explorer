//! Security primitives of the Windows account service (PRM-M7): wide
//! strings, SIDs, security descriptors written in SDDL, the owner of a
//! kernel or file object, the facts of a token (user, elevation, the
//! Administrators group), privileges of this process and local accounts.
//! The worker, the service and the elevated setup share them.
use std::ffi::OsStr;
use std::io;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};

use windows_sys::core::PCWSTR;
use windows_sys::Win32::Foundation::{LocalFree, ERROR_NOT_ALL_ASSIGNED, HANDLE, LUID};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SDDL_REVISION_1, SE_OBJECT_TYPE,
};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, CopySid, CreateWellKnownSid, EqualSid, GetLengthSid,
    GetSecurityDescriptorDacl, GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
    IsValidSid, LookupAccountNameW, LookupPrivilegeValueW, TokenElevation, TokenGroups, TokenUser,
    WinBuiltinAdministratorsSid, ACL, LUID_AND_ATTRIBUTES, OWNER_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, SECURITY_MAX_SID_SIZE, SE_PRIVILEGE_ENABLED,
    SID_AND_ATTRIBUTES, SID_NAME_USE, TOKEN_ADJUST_PRIVILEGES, TOKEN_ELEVATION, TOKEN_GROUPS,
    TOKEN_INFORMATION_CLASS, TOKEN_PRIVILEGES, TOKEN_QUERY, TOKEN_USER, WELL_KNOWN_SID_TYPE,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::System::WindowsProgramming::GetComputerNameW;

use super::exec_handles::OwnedHandle;

/// Longest string form of a SID (`S-1-` and 15 sub-authorities).
const MAX_SID_TEXT: usize = 184;

/// `text` as a NUL-terminated UTF-16 string.
pub(super) fn wide(text: impl AsRef<OsStr>) -> Vec<u16> {
    text.as_ref().encode_wide().chain(Some(0)).collect()
}

/// The UTF-16 string at `pointer` up to its NUL; empty for a null pointer.
///
/// # Safety
/// `pointer` is null or points at a NUL-terminated UTF-16 string.
pub(super) unsafe fn from_wide_ptr(pointer: *const u16) -> String {
    if pointer.is_null() {
        return String::new();
    }
    let mut length = 0;
    while *pointer.add(length) != 0 {
        length += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length))
}

/// `text` has the string form of a SID: `S-1-` and decimal parts.
pub(super) fn sid_text(text: &str) -> bool {
    text.len() <= MAX_SID_TEXT
        && text.starts_with("S-1-")
        && text[4..]
            .split('-')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// A SID in a buffer of its own.
#[derive(Clone, Debug)]
pub(super) struct Sid(Vec<u8>);

impl Sid {
    /// Copies the SID at `sid`.
    ///
    /// # Safety
    /// `sid` is null or points at memory that holds a SID.
    pub(super) unsafe fn copy_from(sid: PSID) -> io::Result<Self> {
        if sid.is_null() || IsValidSid(sid) == 0 {
            return Err(invalid("not a valid SID"));
        }
        let length = GetLengthSid(sid);
        let mut bytes = vec![0u8; length as usize];
        if CopySid(length, bytes.as_mut_ptr().cast(), sid) == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(bytes))
    }

    /// A well-known SID of this computer (`WinLocalSystemSid`, …).
    pub(super) fn well_known(kind: WELL_KNOWN_SID_TYPE) -> io::Result<Self> {
        let mut bytes = vec![0u8; SECURITY_MAX_SID_SIZE as usize];
        let mut size = SECURITY_MAX_SID_SIZE;
        let created =
            unsafe { CreateWellKnownSid(kind, null_mut(), bytes.as_mut_ptr().cast(), &mut size) };
        if created == 0 {
            return Err(io::Error::last_os_error());
        }
        bytes.truncate(size as usize);
        Ok(Self(bytes))
    }

    /// The SID for calls that take one; they never change it.
    pub(super) fn as_psid(&self) -> PSID {
        self.0.as_ptr().cast_mut().cast()
    }

    /// The string form `S-1-…`.
    pub(super) fn text(&self) -> io::Result<String> {
        let mut text: *mut u16 = null_mut();
        if unsafe { ConvertSidToStringSidW(self.as_psid(), &mut text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let value = unsafe { from_wide_ptr(text) };
        unsafe { LocalFree(text.cast()) };
        Ok(value)
    }

    /// The last sub-authority: the relative id (RID) of an account SID.
    pub(super) fn rid(&self) -> u32 {
        // SAFETY: the buffer holds a valid SID (`copy_from`, `well_known`).
        unsafe {
            let count = *GetSidSubAuthorityCount(self.as_psid());
            if count == 0 {
                return 0;
            }
            *GetSidSubAuthority(self.as_psid(), u32::from(count) - 1)
        }
    }
}

impl PartialEq for Sid {
    fn eq(&self, other: &Self) -> bool {
        unsafe { EqualSid(self.as_psid(), other.as_psid()) != 0 }
    }
}

/// A security descriptor made from SDDL, freed on drop.
pub(super) struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

impl SecurityDescriptor {
    pub(super) fn from_sddl(sddl: &str) -> io::Result<Self> {
        let text = wide(sddl);
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        };
        if converted == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(descriptor))
    }

    pub(super) fn raw(&self) -> PSECURITY_DESCRIPTOR {
        self.0
    }

    /// Attributes that create an object with this descriptor; `inherit`
    /// makes its handle inheritable.
    pub(super) fn attributes(&self, inherit: bool) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: i32::from(inherit),
        }
    }

    /// The DACL of the descriptor (it lives as long as the descriptor).
    pub(super) fn dacl(&self) -> io::Result<*mut ACL> {
        let (mut present, mut defaulted) = (0, 0);
        let mut dacl: *mut ACL = null_mut();
        if unsafe { GetSecurityDescriptorDacl(self.0, &mut present, &mut dacl, &mut defaulted) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        if present == 0 || dacl.is_null() {
            return Err(invalid("the security descriptor has no DACL"));
        }
        Ok(dacl)
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { LocalFree(self.0) };
        }
    }
}

/// The owner of the kernel or file object behind `handle`, which needs
/// `READ_CONTROL`.
pub(super) fn owner_of(handle: HANDLE, kind: SE_OBJECT_TYPE) -> io::Result<Sid> {
    let mut owner: PSID = null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    let status = unsafe {
        GetSecurityInfo(
            handle,
            kind,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let copied = unsafe { Sid::copy_from(owner) };
    unsafe { LocalFree(descriptor) };
    copied
}

/// The variable-sized information `class` of `token`, 8-byte aligned.
fn token_information(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> io::Result<Vec<u64>> {
    let mut size = 0u32;
    unsafe { GetTokenInformation(token, class, null_mut(), 0, &mut size) };
    if size == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![0u64; (size as usize).div_ceil(size_of::<u64>())];
    let read =
        unsafe { GetTokenInformation(token, class, buffer.as_mut_ptr().cast(), size, &mut size) };
    if read == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(buffer)
}

/// The user of `token`.
pub(super) fn token_user(token: HANDLE) -> io::Result<Sid> {
    let buffer = token_information(token, TokenUser)?;
    let user = buffer.as_ptr().cast::<TOKEN_USER>();
    // SAFETY: `GetTokenInformation(TokenUser)` filled the buffer with a
    // `TOKEN_USER` whose SID points into the same buffer.
    unsafe { Sid::copy_from((*user).User.Sid) }
}

/// `token` has administrator rights (UAC: an elevated or unfiltered token).
pub(super) fn token_elevated(token: HANDLE) -> io::Result<bool> {
    let mut elevation: TOKEN_ELEVATION = unsafe { zeroed() };
    let mut returned = 0u32;
    let read = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    if read == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(elevation.TokenIsElevated != 0)
}

/// `token` holds the group BUILTIN\Administrators, enabled or for deny only.
pub(super) fn token_has_administrators(token: HANDLE) -> io::Result<bool> {
    let administrators = Sid::well_known(WinBuiltinAdministratorsSid)?;
    let buffer = token_information(token, TokenGroups)?;
    let groups = buffer.as_ptr().cast::<TOKEN_GROUPS>();
    // SAFETY: `GetTokenInformation(TokenGroups)` filled the buffer with a
    // `TOKEN_GROUPS` of `GroupCount` entries whose SIDs point into it.
    unsafe {
        let first = std::ptr::addr_of!((*groups).Groups).cast::<SID_AND_ATTRIBUTES>();
        for index in 0..(*groups).GroupCount as usize {
            if EqualSid((*first.add(index)).Sid, administrators.as_psid()) != 0 {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn process_token(access: u32) -> io::Result<OwnedHandle> {
    let mut token = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    OwnedHandle::new(token)
}

/// The user this process runs as.
pub(super) fn current_user_sid() -> io::Result<Sid> {
    token_user(process_token(TOKEN_QUERY)?.raw())
}

/// This process has administrator rights (an elevated token).
pub(super) fn process_is_elevated() -> bool {
    process_token(TOKEN_QUERY)
        .and_then(|token| token_elevated(token.raw()))
        .unwrap_or(false)
}

/// Enables each privilege of `names` in this process's token; each must
/// be in it (LocalSystem holds those the service needs, disabled).
pub(super) fn enable_privileges(names: &[PCWSTR]) -> io::Result<()> {
    let token = process_token(TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY)?;
    for name in names {
        let mut luid: LUID = unsafe { zeroed() };
        if unsafe { LookupPrivilegeValueW(null(), *name, &mut luid) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let privileges = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };
        let adjusted = unsafe {
            AdjustTokenPrivileges(token.raw(), 0, &privileges, 0, null_mut(), null_mut())
        };
        // The call also succeeds when the token lacks the privilege.
        let error = io::Error::last_os_error();
        if adjusted == 0 || error.raw_os_error() == Some(ERROR_NOT_ALL_ASSIGNED as i32) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("a privilege the account service needs is missing: {error}"),
            ));
        }
    }
    Ok(())
}

/// A local account as Windows resolves its name.
pub(super) struct ResolvedAccount {
    pub(super) sid: Sid,
    /// The domain Windows names for the account; this computer's name for
    /// a local account.
    pub(super) domain: String,
    pub(super) kind: SID_NAME_USE,
}

/// Resolves the account `name` (`LookupAccountNameW`).
pub(super) fn lookup_account(name: &str) -> io::Result<ResolvedAccount> {
    let account = wide(name);
    let (mut sid_size, mut domain_size, mut kind) = (0u32, 0u32, 0 as SID_NAME_USE);
    unsafe {
        LookupAccountNameW(
            null(),
            account.as_ptr(),
            null_mut(),
            &mut sid_size,
            null_mut(),
            &mut domain_size,
            &mut kind,
        )
    };
    if sid_size == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut sid = vec![0u8; sid_size as usize];
    let mut domain = vec![0u16; domain_size.max(1) as usize];
    let found = unsafe {
        LookupAccountNameW(
            null(),
            account.as_ptr(),
            sid.as_mut_ptr().cast(),
            &mut sid_size,
            domain.as_mut_ptr(),
            &mut domain_size,
            &mut kind,
        )
    };
    if found == 0 {
        return Err(io::Error::last_os_error());
    }
    let domain = String::from_utf16_lossy(&domain[..(domain_size as usize).min(domain.len())]);
    Ok(ResolvedAccount {
        sid: Sid(sid),
        domain,
        kind,
    })
}

/// This computer's NetBIOS name, the domain of its local accounts.
pub(super) fn computer_name() -> io::Result<String> {
    let mut buffer = [0u16; 64];
    let mut size = buffer.len() as u32;
    if unsafe { GetComputerNameW(buffer.as_mut_ptr(), &mut size) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(String::from_utf16_lossy(&buffer[..size as usize]))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.to_string())
}
