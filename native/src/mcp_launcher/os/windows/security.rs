//! Reject reparse points and write grants outside owner/Administrators/SYSTEM.
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::*;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ALL_ACCESS, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
};
#[cfg(test)]
#[path = "security_tests.rs"]
mod security_tests;
// Documented ACE_HEADER byte values; independent of binding alias widths.
const ALLOW_ACE: u8 = 0;
const DENY_ACE: u8 = 1;
const AUDIT_ACE: u8 = 2;
const INHERIT_ONLY_FLAG: u8 = 8;

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
struct Descriptor(PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn descriptor(path: &Path) -> Result<(Descriptor, PSID, *mut ACL), String> {
    let mut sd = std::ptr::null_mut();
    let mut owner = std::ptr::null_mut();
    let mut acl = std::ptr::null_mut();
    let result = unsafe {
        GetNamedSecurityInfoW(
            wide(path).as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut acl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if result != 0 {
        return Err(format!("runtime_acl_read {}: {result}", path.display()));
    }
    Ok((Descriptor(sd), owner, acl))
}
pub(super) fn owner(path: &Path) -> Result<Vec<u8>, String> {
    let (_sd, owner, _) = descriptor(path)?;
    if owner.is_null() || unsafe { IsValidSid(owner) } == 0 {
        return Err("runtime_acl_owner_invalid".into());
    }
    if !trusted_owner(owner)? {
        return Err("runtime_installation_owner_untrusted".into());
    }
    let size = unsafe { GetLengthSid(owner) };
    let mut value = vec![0u8; size as usize];
    if unsafe { CopySid(size, value.as_mut_ptr() as _, owner) } == 0 {
        return Err("runtime_acl_owner_copy".into());
    }
    Ok(value)
}
pub(super) fn check(path: &Path, expected_owner: &[u8]) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.file_attributes() & 0x400 != 0 {
        return Err(format!("runtime_reparse_point: {}", path.display()));
    }
    let (_sd, owner, acl) = descriptor(path)?;
    let expected_sid = expected_owner.as_ptr() as PSID;
    let administrative_pair = !owner.is_null()
        && unsafe {
            (IsWellKnownSid(owner, WinLocalSystemSid) != 0
                || IsWellKnownSid(owner, WinBuiltinAdministratorsSid) != 0)
                && (IsWellKnownSid(expected_sid, WinLocalSystemSid) != 0
                    || IsWellKnownSid(expected_sid, WinBuiltinAdministratorsSid) != 0)
        };
    if acl.is_null()
        || owner.is_null()
        || (!administrative_pair && unsafe { EqualSid(owner, expected_sid) } == 0)
    {
        return Err(format!(
            "runtime_acl_owner_or_dacl_invalid: {}",
            path.display()
        ));
    }
    let mut info: ACL_SIZE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe {
        GetAclInformation(
            acl,
            &mut info as *mut _ as _,
            std::mem::size_of_val(&info) as u32,
            AclSizeInformation,
        )
    } == 0
    {
        return Err("runtime_acl_unreadable".into());
    }
    // FILE_WRITE_DATA/APPEND/EA/ATTRIBUTES, DELETE_CHILD, DELETE, WRITE_DAC,
    // WRITE_OWNER. Generic rights are expanded before testing this mask.
    const MUTATING: u32 = 0x0000_0156 | 0x0001_0000 | 0x0004_0000 | 0x0008_0000;
    for index in 0..info.AceCount {
        let mut raw = std::ptr::null_mut();
        if unsafe { GetAce(acl, index, &mut raw) } == 0 {
            return Err("runtime_acl_ace_unreadable".into());
        }
        let header = unsafe { &*(raw as *const ACE_HEADER) };
        // Inherit-only entries do not apply here, but descendants are checked independently.
        if header.AceFlags & INHERIT_ONLY_FLAG != 0 {
            continue;
        }
        if header.AceType == ALLOW_ACE {
            let ace = unsafe { &*(raw as *const ACCESS_ALLOWED_ACE) };
            if file_rights(ace.Mask) & MUTATING != 0 {
                let sid = &ace.SidStart as *const u32 as PSID;
                let trusted = unsafe {
                    EqualSid(sid, owner) != 0
                        || IsWellKnownSid(sid, WinLocalSystemSid) != 0
                        || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0
                };
                if !trusted {
                    return Err(format!(
                        "runtime_acl_foreign_write_grant: {}",
                        path.display()
                    ));
                }
            }
        } else if header.AceType != DENY_ACE && header.AceType != AUDIT_ACE {
            return Err(format!("runtime_acl_ace_unsupported: {}", path.display()));
        }
    }
    Ok(())
}
pub(super) fn protect(path: &Path) -> Result<(), String> {
    // Installation inherits the installer's protected root ACL. Fail rather
    // than silently repairing an attacker-writable installation at runtime.
    let value = owner(path)?;
    check(path, &value)
}

fn trusted_owner(owner: PSID) -> Result<bool, String> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    if unsafe {
        IsWellKnownSid(owner, WinLocalSystemSid) != 0
            || IsWellKnownSid(owner, WinBuiltinAdministratorsSid) != 0
    } {
        return Ok(true);
    }
    let mut raw = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let token = unsafe { OwnedHandle::from_raw_handle(raw as _) };
    let mut length = 0;
    unsafe {
        GetTokenInformation(
            token.as_raw_handle() as _,
            TokenUser,
            std::ptr::null_mut(),
            0,
            &mut length,
        );
    }
    let mut data = vec![0u8; length as usize];
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle() as _,
            TokenUser,
            data.as_mut_ptr() as _,
            length,
            &mut length,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let user = unsafe { std::ptr::read_unaligned(data.as_ptr() as *const TOKEN_USER) };
    Ok(unsafe { EqualSid(owner, user.User.Sid) != 0 } || trusted_installer(owner))
}

pub(super) fn check_ancestor(path: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.file_attributes() & 0x400 != 0 {
        return Err(format!("runtime_ancestor_reparse: {}", path.display()));
    }
    let (_sd, owner, acl) = descriptor(path)?;
    if owner.is_null() || acl.is_null() {
        return Err("runtime_ancestor_acl_missing".into());
    }
    // An object's owner can change its DACL even without an explicit allow ACE.
    if !trusted_owner(owner)? {
        return Err(format!("runtime_ancestor_owner_untrusted: {}", path.display()));
    }
    let mut info: ACL_SIZE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe {
        GetAclInformation(
            acl,
            &mut info as *mut _ as _,
            std::mem::size_of_val(&info) as u32,
            AclSizeInformation,
        )
    } == 0
    {
        return Err("runtime_ancestor_acl_unreadable".into());
    }
    // Creation of a fresh sibling is harmless; replacement/deletion and ACL
    // ownership changes of this existing path would break the root binding.
    // Every real ancestor contains the protected root, so it cannot be turned
    // into a new reparse point without first deleting that protected child.
    const REPLACING: u32 = 0x0000_0040 | 0x0001_0000 | 0x0004_0000 | 0x0008_0000;
    for index in 0..info.AceCount {
        let mut raw = std::ptr::null_mut();
        if unsafe { GetAce(acl, index, &mut raw) } == 0 {
            return Err("runtime_ancestor_ace_unreadable".into());
        }
        let header = unsafe { &*(raw as *const ACE_HEADER) };
        if header.AceFlags & INHERIT_ONLY_FLAG != 0 {
            continue;
        }
        if header.AceType == ALLOW_ACE {
            let ace = unsafe { &*(raw as *const ACCESS_ALLOWED_ACE) };
            let rights = file_rights(ace.Mask);
            if rights & REPLACING == 0 {
                continue;
            }
            let sid = &ace.SidStart as *const u32 as PSID;
            // A standard user's private Temp may be administratively owned
            // while granting that current user full access. Trust the actual
            // token user, never every trustee merely because the owner is trusted.
            if !trusted_owner(sid)? {
                return Err(format!(
                    "runtime_ancestor_replace_grant: {} (mask={:#010x}, mapped={rights:#010x}, trustee={})",
                    path.display(), ace.Mask, sid_text(sid)
                ));
            }
        } else if header.AceType != DENY_ACE && header.AceType != AUDIT_ACE {
            return Err(format!(
                "runtime_ancestor_ace_unsupported: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn file_rights(mut mask: u32) -> u32 {
    let mapping = GENERIC_MAPPING {
        GenericRead: FILE_GENERIC_READ,
        GenericWrite: FILE_GENERIC_WRITE,
        GenericExecute: FILE_GENERIC_EXECUTE,
        GenericAll: FILE_ALL_ACCESS,
    };
    unsafe { MapGenericMask(&mut mask, &mapping) };
    mask
}

fn sid_text(sid: PSID) -> String {
    let mut raw = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut raw) } == 0 {
        return "unavailable".into();
    }
    let mut length = 0;
    while length < 256 && unsafe { *raw.add(length) } != 0 {
        length += 1;
    }
    let text = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(raw, length) });
    unsafe { LocalFree(raw as _) };
    text
}

fn trusted_installer(owner: PSID) -> bool {
    // Resolve the OS service namespace rather than accepting an arbitrary
    // service SID or relying on a guessed hard-coded account SID.
    let name = "NT SERVICE\\TrustedInstaller"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut sid_bytes = 0;
    let mut domain_len = 0;
    let mut usage = 0;
    unsafe {
        LookupAccountNameW(
            std::ptr::null(),
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut sid_bytes,
            std::ptr::null_mut(),
            &mut domain_len,
            &mut usage,
        );
    }
    if sid_bytes == 0 || sid_bytes > 65536 || domain_len > 65536 {
        return false;
    }
    let mut sid = vec![0u64; (sid_bytes as usize).div_ceil(8)];
    let mut domain = vec![0u16; domain_len as usize];
    unsafe {
        LookupAccountNameW(
            std::ptr::null(),
            name.as_ptr(),
            sid.as_mut_ptr() as _,
            &mut sid_bytes,
            domain.as_mut_ptr(),
            &mut domain_len,
            &mut usage,
        ) != 0
            && EqualSid(owner, sid.as_ptr() as _) != 0
    }
}
