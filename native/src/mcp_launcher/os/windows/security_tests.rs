//! Actual isolated DACLs; no mutation of the runner's existing ancestors.
use super::*;

fn set_dacl(path: &Path, extra: &str) {
    let sid = sid_text(owner(path).unwrap().as_ptr() as _);
    let sddl = format!("D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA){extra}")
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut descriptor = std::ptr::null_mut();
    assert_ne!(unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(), 1, &mut descriptor, std::ptr::null_mut(),
        )
    }, 0);
    let descriptor = Descriptor(descriptor);
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl = std::ptr::null_mut();
    assert_ne!(unsafe {
        GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted)
    }, 0);
    assert_ne!(present, 0);
    assert_eq!(unsafe {
        SetNamedSecurityInfoW(
            wide(path).as_ptr(), SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(), std::ptr::null_mut(), acl, std::ptr::null_mut(),
        )
    }, 0);
}

#[test]
fn batches_p13_windows_ancestor_creation_rights_do_not_authorize_owned_file_mutation() {
    let root = tempfile::tempdir().unwrap();
    for grant in ["(A;;GW;;;BU)", "(A;;0x00000116;;;BU)"] {
        set_dacl(root.path(), grant);
        check_ancestor(root.path()).unwrap();
        let expected_owner = owner(root.path()).unwrap();
        assert!(check(root.path(), &expected_owner)
            .unwrap_err().starts_with("runtime_acl_foreign_write_grant:"));
    }
    const REPLACING: u32 = 0x0000_0040 | 0x0001_0000 | 0x0004_0000 | 0x0008_0000;
    assert_eq!(file_rights(0x4000_0000) & REPLACING, 0);
    assert_eq!(file_rights(0x1000_0000) & REPLACING, REPLACING);
}

#[test]
fn batches_p13_windows_ancestor_foreign_replacement_rights_remain_rejected() {
    let root = tempfile::tempdir().unwrap();
    for grant in [
        "(A;;0x00000040;;;BU)",
        "(A;;SD;;;BU)",
        "(A;;WD;;;BU)",
        "(A;;WO;;;BU)",
        "(A;;GA;;;BU)",
    ] {
        set_dacl(root.path(), grant);
        let error = check_ancestor(root.path()).unwrap_err();
        assert!(error.starts_with("runtime_ancestor_replace_grant:"), "{error}");
        assert!(error.contains("trustee=S-1-5-32-545"), "{error}");
    }
    // A grant only for future children cannot replace this ancestor itself.
    set_dacl(root.path(), "(A;OIIO;GA;;;BU)");
    check_ancestor(root.path()).unwrap();
}

#[test]
fn batches_p13_windows_ancestor_trust_follows_token_user_not_object_owner() {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    let root = tempfile::tempdir().unwrap();
    let mut raw = std::ptr::null_mut();
    assert_ne!(unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) }, 0);
    let token = unsafe { OwnedHandle::from_raw_handle(raw as _) };
    let mut bytes = 0;
    unsafe { GetTokenInformation(token.as_raw_handle() as _, TokenUser, std::ptr::null_mut(), 0, &mut bytes); }
    let mut storage = vec![0u64; (bytes as usize).div_ceil(8)];
    assert_ne!(unsafe {
        GetTokenInformation(token.as_raw_handle() as _, TokenUser, storage.as_mut_ptr() as _, bytes, &mut bytes)
    }, 0);
    let user = unsafe { &*(storage.as_ptr() as *const TOKEN_USER) };
    assert!(trusted_owner(user.User.Sid).unwrap());
    set_dacl(root.path(), "");
    check_ancestor(root.path()).unwrap();
    // Neither BUILTIN Users nor Everyone becomes trusted through an admin owner.
    for kind in [WinBuiltinUsersSid, WinWorldSid] {
        let mut sid = [0u64; 16];
        let mut length = std::mem::size_of_val(&sid) as u32;
        assert_ne!(unsafe {
            CreateWellKnownSid(kind, std::ptr::null_mut(), sid.as_mut_ptr() as _, &mut length)
        }, 0);
        assert!(!trusted_owner(sid.as_mut_ptr() as _).unwrap());
    }
}
