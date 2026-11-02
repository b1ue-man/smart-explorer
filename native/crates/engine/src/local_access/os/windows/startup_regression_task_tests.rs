//! Real existing Windows data: a protected DACL's owner is accepted only for the
//! effective user or the default owner. The repair test of this file moved to
//! daemon/os/shared/startup_regression_task_access_tests.rs (plan D13).
use super::*;

#[test]
fn startup_regression_task_only_effective_user_or_default_owner_is_accepted() {
    use windows_sys::Win32::Security::{CreateWellKnownSid, WinWorldSid};
    let security = PrivateSecurity::new(true).unwrap();
    assert!(security.accepts_owner(security.sid()));
    let default_owner = unsafe { (*(security.default_owner.as_ptr().cast::<TOKEN_OWNER>())).Owner };
    assert!(security.accepts_owner(default_owner));
    let mut sid = [0u64; 16];
    let mut bytes = std::mem::size_of_val(&sid) as u32;
    win(unsafe {
        CreateWellKnownSid(
            WinWorldSid,
            std::ptr::null_mut(),
            sid.as_mut_ptr().cast(),
            &mut bytes,
        )
    })
    .unwrap();
    assert!(!security.accepts_owner(sid.as_mut_ptr().cast()));
    assert!(!security.accepts_owner(std::ptr::null_mut()));
}
