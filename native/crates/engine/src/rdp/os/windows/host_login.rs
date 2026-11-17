//! Windows side of the sign-in with the grant's account (plan 2026-11-17
//! P4): the Windows name of the grant's account (assumption K2), the check
//! of a typed password (`LogonUserW`, network logon, decision 2) and the
//! automatic sign-in of `se-remote`, whose password the elevated account
//! service sets anew for every session (decision 5). Every other account
//! signs in with the password the viewer typed once.
use crate::rdp::core::{windows_account, HostAccount, LoginInfo, LoginState};
use crate::rdp::os::shared::host_login::{Automatic, Viewer};
use crate::share::access_grant::{AccountProfile, REMOTE_ACCOUNT_NAME};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_LOGON_FAILURE, HANDLE};

/// The one RDP service of a Windows host (`probe.rs`).
const SERVICE: &str = "windows-rdp";

pub fn supports(service_id: &str) -> Result<(), String> {
    if service_id == SERVICE {
        Ok(())
    } else {
        Err(format!("RDP-Dienst {service_id} gibt es auf Windows nicht"))
    }
}

/// The account of the grant as RDP clients name it: the worker's own
/// account, `se-remote` or the named local account.
pub fn account(profile: &AccountProfile) -> Result<HostAccount, String> {
    match profile {
        AccountProfile::Own => {
            let sam = own_sam_name()?;
            let principal = match sam.split_once('\\') {
                Some((domain, user)) => local_principal(domain, user),
                None => None,
            };
            windows_account(&sam, principal.as_deref())
        }
        AccountProfile::Remote => named(REMOTE_ACCOUNT_NAME),
        AccountProfile::Account { name } => named(name),
    }
}

/// `se-remote` signs in automatically: the account service sets a new
/// password for this session and admits the account to Remote Desktop.
pub fn automatic(
    _service_id: &str,
    profile: &AccountProfile,
    _account: &HostAccount,
    _viewer: &Viewer,
    status_only: bool,
) -> Automatic {
    if *profile != AccountProfile::Remote {
        return Automatic::No;
    }
    if !crate::share::exec_run_as_supported() {
        return Automatic::Unsupported(
            "der Kontodienst für se-remote ist auf diesem Gerät nicht eingerichtet (`se share exec accounts setup --remote-account` als Administrator)".into(),
        );
    }
    if status_only {
        return Automatic::Available;
    }
    match crate::share::exec_remote_rdp_password(true) {
        Ok(secret) => Automatic::Secret(secret),
        Err(error) => Automatic::Unsupported(format!(
            "der Kontodienst setzt kein Kennwort für se-remote ({error}); ein Kontodienst vor 0.5.198 braucht `se share exec accounts update`"
        )),
    }
}

/// Checks `password` with a network logon (`LOGON32_LOGON_NETWORK`): `false`
/// for a wrong password, an error when Windows refuses for another reason
/// (locked, expired, logon type not granted).
pub fn verify(account: &HostAccount, password: &str) -> Result<bool, String> {
    use windows_sys::Win32::Security::{
        LogonUserW, LOGON32_LOGON_NETWORK, LOGON32_PROVIDER_DEFAULT,
    };
    let user = wide(account.username());
    let domain = account.domain().map(wide);
    let mut secret = wide(password);
    let mut token: HANDLE = null_mut();
    // SAFETY: terminated buffers live through the call; `token` is a valid
    // output location.
    let done = unsafe {
        LogonUserW(
            user.as_ptr(),
            domain.as_ref().map_or(null(), |domain| domain.as_ptr()),
            secret.as_ptr(),
            LOGON32_LOGON_NETWORK,
            LOGON32_PROVIDER_DEFAULT,
            &mut token,
        )
    };
    let error = std::io::Error::last_os_error();
    secret.fill(0);
    if done != 0 {
        // SAFETY: a successful LogonUserW returned this token handle.
        unsafe { CloseHandle(token) };
        return Ok(true);
    }
    if error.raw_os_error() == Some(ERROR_LOGON_FAILURE as i32) {
        return Ok(false);
    }
    Err(format!(
        "Windows prüft das Kennwort von {} nicht: {error}",
        account.qualified()
    ))
}

/// A right that ended during the request: a se-remote password it issued
/// is replaced by one nobody receives.
pub fn withdraw(_viewer: &Viewer, issued: &LoginInfo) {
    if issued.state == LoginState::Automatic && issued.secret.is_some() {
        rotate_remote();
    }
}

/// „Bildschirm steuern“ ended for a viewer: `se-remote` gets a password
/// nobody receives (decision 7).
pub fn coverage_ended(_steers: &dyn Fn(&Viewer) -> bool, _steering_remains: bool) {
    rotate_remote();
}

fn rotate_remote() {
    if !crate::share::exec_run_as_supported() {
        return;
    }
    if let Err(error) = crate::share::exec_remote_rdp_password(false) {
        eprintln!("RDP-Anmeldung: Kennwort von se-remote nicht erneuert: {error}");
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// `DOMAIN\user` of this worker's account (`GetUserNameExW`,
/// `NameSamCompatible`).
fn own_sam_name() -> Result<String, String> {
    use windows_sys::Win32::Security::Authentication::Identity::{
        GetUserNameExW, NameSamCompatible,
    };
    let mut buffer = vec![0u16; 512];
    let mut size = buffer.len() as u32;
    // SAFETY: the buffer is writable for `size` characters.
    if unsafe { GetUserNameExW(NameSamCompatible, buffer.as_mut_ptr(), &mut size) } == 0 {
        return Err(format!(
            "Konto des Hintergrunddienstes nicht ermittelbar: {}",
            std::io::Error::last_os_error()
        ));
    }
    let length = (size as usize).min(buffer.len());
    Ok(String::from_utf16_lossy(&buffer[..length]))
}

/// `DOMAIN\name` of a local account (`LookupAccountNameW`).
fn named(name: &str) -> Result<HostAccount, String> {
    use windows_sys::Win32::Security::{LookupAccountNameW, SID_NAME_USE};
    let account = wide(name);
    let (mut sid_size, mut domain_size, mut kind) = (0u32, 0u32, 0 as SID_NAME_USE);
    // SAFETY: the size query writes only the three outputs.
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
        return Err(format!(
            "Konto {name} gibt es auf diesem Gerät nicht: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut sid = vec![0u8; sid_size as usize];
    let mut domain = vec![0u16; domain_size.max(1) as usize];
    // SAFETY: both buffers are writable for the sizes the first call named.
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
        return Err(format!(
            "Konto {name} nicht auflösbar: {}",
            std::io::Error::last_os_error()
        ));
    }
    let domain = String::from_utf16_lossy(&domain[..(domain_size as usize).min(domain.len())]);
    windows_account(
        &format!("{domain}\\{name}"),
        local_principal(&domain, name).as_deref(),
    )
}

/// The Microsoft account of a local account (its domain is this computer's
/// name); `None` for a domain account.
fn local_principal(domain: &str, user: &str) -> Option<String> {
    use windows_sys::Win32::System::WindowsProgramming::GetComputerNameW;
    let mut buffer = [0u16; 64];
    let mut size = buffer.len() as u32;
    // SAFETY: the buffer is writable for `size` characters.
    if unsafe { GetComputerNameW(buffer.as_mut_ptr(), &mut size) } == 0 {
        return None;
    }
    let computer = String::from_utf16_lossy(&buffer[..(size as usize).min(buffer.len())]);
    if !domain.eq_ignore_ascii_case(&computer) {
        return None;
    }
    microsoft_principal(user)
}

/// The Microsoft account a local account is linked to (`NetUserGetInfo`
/// level 24, `usri24_internet_principal_name`); `None` for a plain local
/// account.
fn microsoft_principal(user: &str) -> Option<String> {
    use windows_sys::Win32::NetworkManagement::NetManagement::{
        NERR_Success, NetApiBufferFree, NetUserGetInfo, USER_INFO_24,
    };
    let name = wide(user);
    let mut buffer: *mut u8 = null_mut();
    // SAFETY: level 24 fills `buffer` with one USER_INFO_24 the call
    // allocates; it is freed below.
    let status = unsafe { NetUserGetInfo(null(), name.as_ptr(), 24, &mut buffer) };
    if status != NERR_Success || buffer.is_null() {
        if !buffer.is_null() {
            // SAFETY: allocated by NetUserGetInfo.
            unsafe { NetApiBufferFree(buffer as *const core::ffi::c_void) };
        }
        return None;
    }
    // SAFETY: a successful level-24 call returned one USER_INFO_24.
    let info = unsafe { &*buffer.cast::<USER_INFO_24>() };
    let principal = (info.usri24_internet_identity != 0
        && !info.usri24_internet_principal_name.is_null())
    .then(|| {
        // SAFETY: a non-null principal is a NUL-terminated wide string in
        // the buffer, read before it is freed.
        unsafe { wide_to_string(info.usri24_internet_principal_name) }
    })
    .filter(|principal| !principal.is_empty());
    // SAFETY: allocated by NetUserGetInfo, freed once.
    unsafe { NetApiBufferFree(buffer as *const core::ffi::c_void) };
    principal
}

/// # Safety
/// `text` points to a NUL-terminated UTF-16 string.
unsafe fn wide_to_string(text: *const u16) -> String {
    let mut length = 0usize;
    while *text.add(length) != 0 {
        length += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(text, length))
}

#[cfg(test)]
#[path = "rdp_konto_task_tests.rs"]
mod rdp_konto_task_tests;
