//! Logon without password (S4U) of the Windows account service (C2, NR
//! N1). The service, LocalSystem with `SeTcbPrivilege`, registers as a logon
//! process and logs a local account on with `MsV1_0S4ULogon` and the logon
//! type `Network`; for a relay the impersonation token becomes a primary
//! token. The probe (D.2.7) does the same logon without a process and
//! measures what the token may do, so no account list rests on an
//! assumption about Microsoft accounts, Entra or Windows Home (A6, A7).
//! Only local user accounts of this PC are targets. System accounts, the
//! built-in Administrator and every account whose token is elevated or
//! holds the Administrators group (also for deny only) are refused: the
//! service never runs anything as SYSTEM or as an administrator.
use std::io;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HANDLE, LUID};
use windows_sys::Win32::Security::Authentication::Identity::{
    LsaDeregisterLogonProcess, LsaFreeReturnBuffer, LsaLogonUser, LsaLookupAuthenticationPackage,
    LsaNtStatusToWinError, LsaRegisterLogonProcess, MsV1_0S4ULogon, LSA_STRING, LSA_UNICODE_STRING,
    MSV1_0_S4U_LOGON, MSV1_0_S4U_LOGON_FLAG_CHECK_LOGONHOURS, SECURITY_LOGON_TYPE,
};
use windows_sys::Win32::Security::{
    AllocateLocallyUniqueId, DuplicateTokenEx, SecurityImpersonation, SidTypeUser, TokenPrimary,
    QUOTA_LIMITS, TOKEN_ALL_ACCESS, TOKEN_SOURCE,
};

use crate::share::exec_helper_protocol::{HelperErrorCode, HelperReply};

use super::exec_account_config::allowable_name;
use super::exec_account_security::{
    computer_name, lookup_account, token_elevated, token_has_administrators, Sid,
};
use super::exec_handles::OwnedHandle;

/// Relative id of the built-in Administrator account.
const ADMINISTRATOR_RID: u32 = 500;
const LOGON_PROCESS: &[u8] = b"Smart Explorer";
const MSV1_0_PACKAGE: &[u8] = b"MICROSOFT_AUTHENTICATION_PACKAGE_V1_0";
const TOKEN_SOURCE_NAME: [u8; 8] = *b"SmartExp";
/// Sub-statuses of a logon whose account must change its password first.
const STATUS_PASSWORD_EXPIRED: i32 = 0xC000_0071_u32 as i32;
const STATUS_PASSWORD_MUST_CHANGE: i32 = 0xC000_0224_u32 as i32;

/// This service's registration as a logon process.
pub(super) struct Lsa {
    handle: HANDLE,
    package: u32,
}

// SAFETY: the LSA handle is a kernel handle that any thread may use; the
// service keeps the registration in a `Mutex`, so one logon runs at a time.
unsafe impl Send for Lsa {}

impl Lsa {
    /// Registers this process as a logon process (needs `SeTcbPrivilege`).
    pub(super) fn connect() -> io::Result<Self> {
        let mut name = LOGON_PROCESS.to_vec();
        let process = lsa_string(&mut name);
        let mut handle: HANDLE = null_mut();
        let mut mode = 0u32;
        let status = unsafe { LsaRegisterLogonProcess(&process, &mut handle, &mut mode) };
        check(status, "registering as a logon process")?;
        let mut lsa = Self { handle, package: 0 };
        let mut package_name = MSV1_0_PACKAGE.to_vec();
        let package = lsa_string(&mut package_name);
        let status = unsafe { LsaLookupAuthenticationPackage(handle, &package, &mut lsa.package) };
        check(status, "looking up the MSV1_0 package")?;
        Ok(lsa)
    }

    /// The impersonation token of `account`, logged on without password.
    fn logon(&self, account: &CheckedAccount) -> Result<OwnedHandle, LogonError> {
        let (buffer, length) = s4u_logon(&account.name, &account.domain);
        let mut origin_name = LOGON_PROCESS.to_vec();
        let origin = lsa_string(&mut origin_name);
        let mut source: TOKEN_SOURCE = unsafe { zeroed() };
        source.SourceName = TOKEN_SOURCE_NAME.map(|byte| byte as i8);
        unsafe { AllocateLocallyUniqueId(&mut source.SourceIdentifier) };
        let mut profile: *mut core::ffi::c_void = null_mut();
        let mut profile_length = 0u32;
        let mut logon_id: LUID = unsafe { zeroed() };
        let mut token: HANDLE = null_mut();
        let mut quotas: QUOTA_LIMITS = unsafe { zeroed() };
        let mut sub_status = 0i32;
        let status = unsafe {
            LsaLogonUser(
                self.handle,
                &origin,
                SECURITY_LOGON_TYPE::Network,
                self.package,
                buffer.as_ptr().cast(),
                length,
                null(),
                &source,
                &mut profile,
                &mut profile_length,
                &mut logon_id,
                &mut token,
                &mut quotas,
                &mut sub_status,
            )
        };
        if !profile.is_null() {
            unsafe { LsaFreeReturnBuffer(profile) };
        }
        if status != 0 {
            return Err(LogonError { status, sub_status });
        }
        OwnedHandle::new(token).map_err(|_| LogonError {
            status: 0xC000_0001_u32 as i32,
            sub_status: 0,
        })
    }
}

impl Drop for Lsa {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { LsaDeregisterLogonProcess(self.handle) };
        }
    }
}

fn lsa_string(bytes: &mut [u8]) -> LSA_STRING {
    let length = bytes.len().min(usize::from(u16::MAX)) as u16;
    LSA_STRING {
        Length: length,
        MaximumLength: length,
        Buffer: bytes.as_mut_ptr(),
    }
}

/// `MSV1_0_S4U_LOGON` with its two strings right behind it in one buffer,
/// as the LSA copies the buffer and its strings together; 8-byte aligned.
fn s4u_logon(user: &str, domain: &str) -> (Vec<u64>, u32) {
    let user: Vec<u16> = user.encode_utf16().collect();
    let domain: Vec<u16> = domain.encode_utf16().collect();
    let header = size_of::<MSV1_0_S4U_LOGON>();
    let (user_bytes, domain_bytes) = (user.len() * 2, domain.len() * 2);
    let total = header + user_bytes + domain_bytes;
    let mut buffer = vec![0u64; total.div_ceil(size_of::<u64>())];
    let base = buffer.as_mut_ptr().cast::<u8>();
    // SAFETY: the buffer holds `total` bytes: the header, then both strings;
    // the header is written at its 8-byte aligned start.
    unsafe {
        let user_at = base.add(header).cast::<u16>();
        std::ptr::copy_nonoverlapping(user.as_ptr(), user_at, user.len());
        let domain_at = base.add(header + user_bytes).cast::<u16>();
        std::ptr::copy_nonoverlapping(domain.as_ptr(), domain_at, domain.len());
        base.cast::<MSV1_0_S4U_LOGON>().write(MSV1_0_S4U_LOGON {
            MessageType: MsV1_0S4ULogon,
            Flags: MSV1_0_S4U_LOGON_FLAG_CHECK_LOGONHOURS,
            UserPrincipalName: LSA_UNICODE_STRING {
                Length: user_bytes as u16,
                MaximumLength: user_bytes as u16,
                Buffer: user_at,
            },
            DomainName: LSA_UNICODE_STRING {
                Length: domain_bytes as u16,
                MaximumLength: domain_bytes as u16,
                Buffer: domain_at,
            },
        });
    }
    (buffer, total as u32)
}

fn check(status: i32, what: &str) -> io::Result<()> {
    if status == 0 {
        return Ok(());
    }
    let code = unsafe { LsaNtStatusToWinError(status) };
    Err(io::Error::new(
        io::Error::from_raw_os_error(code as i32).kind(),
        format!("{what}: {}", io::Error::from_raw_os_error(code as i32)),
    ))
}

/// A refused logon.
struct LogonError {
    status: i32,
    sub_status: i32,
}

impl LogonError {
    /// The Windows error, with the sub-status where Windows gave one.
    fn text(&self) -> String {
        let code = unsafe { LsaNtStatusToWinError(self.status) };
        let error = io::Error::from_raw_os_error(code as i32);
        if self.sub_status == 0 {
            error.to_string()
        } else {
            format!("{error}; status {:#010x}", self.sub_status as u32)
        }
    }

    fn for_start(&self, name: &str) -> (HelperErrorCode, String) {
        let code = match self.sub_status {
            STATUS_PASSWORD_EXPIRED | STATUS_PASSWORD_MUST_CHANGE => {
                HelperErrorCode::PasswordChangeRequired
            }
            _ => HelperErrorCode::LogonFailed,
        };
        (code, logon_refused(name, &self.text()))
    }
}

/// C6: "Windows lehnt die Anmeldung ohne Passwort für {Name} ab ({Fehler})."
pub(super) fn logon_refused(name: &str, error: &str) -> String {
    format!("Windows refuses the logon without password for {name} ({error})")
}

fn administrator(name: &str) -> String {
    format!(
        "{name} is an administrator or system account; the account service never runs \
         commands as an administrator"
    )
}

/// A local user account of this PC the service may log on.
pub(super) struct CheckedAccount {
    pub(super) name: String,
    domain: String,
    pub(super) sid: Sid,
}

/// Why an account is no target.
pub(super) enum Refused {
    /// An administrator or system account.
    Administrator(String),
    Unknown(String),
    Failed(String),
}

impl Refused {
    pub(super) fn reply(self) -> (HelperErrorCode, String) {
        match self {
            Self::Administrator(detail) => (HelperErrorCode::NotAllowed, detail),
            Self::Unknown(detail) => (HelperErrorCode::UnknownAccount, detail),
            Self::Failed(detail) => (HelperErrorCode::StartFailed, detail),
        }
    }
}

/// `name` is a local user account of this PC and none the service never
/// runs as.
pub(super) fn check_account(name: &str) -> Result<CheckedAccount, Refused> {
    if !allowable_name(name) {
        return Err(Refused::Administrator(administrator(name)));
    }
    let resolved = lookup_account(name).map_err(|error| {
        Refused::Unknown(format!(
            "account {name} does not exist on this PC ({error})"
        ))
    })?;
    let local = computer_name().map_err(|error| Refused::Failed(error.to_string()))?;
    if resolved.kind != SidTypeUser || !resolved.domain.eq_ignore_ascii_case(&local) {
        return Err(Refused::Unknown(format!(
            "{name} is no local user account of this PC"
        )));
    }
    if resolved.sid.rid() == ADMINISTRATOR_RID {
        return Err(Refused::Administrator(administrator(name)));
    }
    Ok(CheckedAccount {
        name: name.to_string(),
        domain: resolved.domain,
        sid: resolved.sid,
    })
}

/// The administrator rights of a token (C2): elevated, and member of the
/// Administrators group (enabled or for deny only).
pub(super) fn token_level(token: HANDLE) -> io::Result<(bool, bool)> {
    Ok((token_elevated(token)?, token_has_administrators(token)?))
}

/// The probe of D.2.7 for an allowed account: a logon without a process
/// and what its token may do.
pub(super) fn probe(lsa: &Lsa, name: &str) -> HelperReply {
    let refused = |elevated, admin_group, detail| HelperReply::Probe {
        ok: false,
        elevated,
        admin_group,
        detail: Some(detail),
    };
    let account = match check_account(name) {
        Ok(account) => account,
        Err(Refused::Administrator(detail)) => return refused(false, true, detail),
        Err(other) => {
            let (code, detail) = other.reply();
            return HelperReply::Error { code, detail };
        }
    };
    let token = match lsa.logon(&account) {
        Ok(token) => token,
        Err(error) => return refused(false, false, logon_refused(name, &error.text())),
    };
    match token_level(token.raw()) {
        Ok((false, false)) => HelperReply::Probe {
            ok: true,
            elevated: false,
            admin_group: false,
            detail: None,
        },
        Ok((elevated, admin_group)) => refused(elevated, admin_group, administrator(name)),
        Err(error) => HelperReply::Error {
            code: HelperErrorCode::StartFailed,
            detail: error.to_string(),
        },
    }
}

/// The primary token of a session as `account`; an administrator token is
/// refused (C2).
pub(super) fn session_token(
    lsa: &Lsa,
    account: &CheckedAccount,
) -> Result<OwnedHandle, (HelperErrorCode, String)> {
    let token = lsa
        .logon(account)
        .map_err(|error| error.for_start(&account.name))?;
    let failed = |error: io::Error| (HelperErrorCode::StartFailed, error.to_string());
    match token_level(token.raw()).map_err(failed)? {
        (false, false) => {}
        _ => return Err((HelperErrorCode::NotAllowed, administrator(&account.name))),
    }
    let mut primary: HANDLE = null_mut();
    let duplicated = unsafe {
        DuplicateTokenEx(
            token.raw(),
            TOKEN_ALL_ACCESS,
            null(),
            SecurityImpersonation,
            TokenPrimary,
            &mut primary,
        )
    };
    if duplicated == 0 {
        return Err(failed(io::Error::last_os_error()));
    }
    OwnedHandle::new(primary).map_err(failed)
}
