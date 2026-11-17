//! The account service sets the password of `se-remote` for Remote Desktop
//! (plan 2026-11-17 P4, decision 5): a new random password for every
//! session, set as LocalSystem (`NetUserSetInfo` level 1003). With `for_rdp`
//! the account also joins the group Remote Desktop Users, without which
//! Windows refuses its RDP logon on a workstation. The password goes only
//! to the registered worker that asked, never to a file or the log; Exec
//! keeps logging on without password (S4U) and is not affected.
use std::io;
use std::ptr::null;

use windows_sys::Win32::Foundation::ERROR_MEMBER_IN_ALIAS;
use windows_sys::Win32::NetworkManagement::NetManagement::{
    NERR_Success, NetLocalGroupAddMembers, NetUserSetInfo, LOCALGROUP_MEMBERS_INFO_0,
    USER_INFO_1003,
};
use windows_sys::Win32::Security::WinBuiltinRemoteDesktopUsersSid;

use crate::share::access_grant::REMOTE_ACCOUNT_NAME;

use super::exec_account_install::{account_name_of, random_password};
use super::exec_account_security::{lookup_account, wide, Sid};

/// Sets a new random password of `se-remote` and returns it; `for_rdp`
/// admits the account to Remote Desktop first.
pub(super) fn new_remote_password(for_rdp: bool) -> io::Result<String> {
    if for_rdp {
        admit_to_remote_desktop()?;
    }
    let password = random_password()?;
    let name = wide(REMOTE_ACCOUNT_NAME);
    let mut secret = wide(&password);
    let info = USER_INFO_1003 {
        usri1003_password: secret.as_mut_ptr(),
    };
    let mut parameter = 0u32;
    // SAFETY: level 1003 reads one USER_INFO_1003 whose password buffer is
    // terminated and lives through the call.
    let status = unsafe {
        NetUserSetInfo(
            null(),
            name.as_ptr(),
            1003,
            (&info as *const USER_INFO_1003).cast(),
            &mut parameter,
        )
    };
    secret.iter_mut().for_each(|unit| *unit = 0);
    if status != NERR_Success {
        return Err(io::Error::other(format!(
            "NetUserSetInfo (se-remote): NET_API_STATUS {status}"
        )));
    }
    Ok(password)
}

/// `se-remote` in the group Remote Desktop Users (localized name of the
/// well-known SID); a member already is no error.
fn admit_to_remote_desktop() -> io::Result<()> {
    let group = wide(account_name_of(&Sid::well_known(
        WinBuiltinRemoteDesktopUsersSid,
    )?)?);
    let member = lookup_account(REMOTE_ACCOUNT_NAME)?.sid;
    let entry = LOCALGROUP_MEMBERS_INFO_0 {
        lgrmi0_sid: member.as_psid(),
    };
    // SAFETY: one LOCALGROUP_MEMBERS_INFO_0 whose SID lives through the call.
    let status = unsafe {
        NetLocalGroupAddMembers(
            null(),
            group.as_ptr(),
            0,
            (&entry as *const LOCALGROUP_MEMBERS_INFO_0).cast(),
            1,
        )
    };
    if status != NERR_Success && status != ERROR_MEMBER_IN_ALIAS {
        return Err(io::Error::other(format!(
            "NetLocalGroupAddMembers (Remote Desktop Users): NET_API_STATUS {status}"
        )));
    }
    Ok(())
}
