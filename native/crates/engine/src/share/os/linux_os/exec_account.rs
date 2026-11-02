//! Local accounts of a Linux host for Exec (PRM-M5, PRM-M6). A grant's
//! run-as account is resolved (`getpwnam_r(3)`, `getgrouplist(3)`) for the
//! job's environment and for the identity check of its supervisor. Before an
//! account unit starts, the account passes the checks of this file: a name
//! systemd reads as a name, an existing account other than root, and a
//! shadow entry, where this process may read it, that is neither expired nor
//! waiting for a password change (as `pam_unix` reads it; PAM decides again
//! in the unit). The account choices (`exec_remote_account.rs`) show the
//! warnings and facts of this file.
use std::ffi::{CStr, CString};
use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::share::access_grant::valid_account_name;
use crate::share::exec_client_messages::{ACCOUNT_HELPER_UNAVAILABLE, ACCOUNT_NOT_ALLOWED};
use crate::share::exec_helper_protocol::{
    account_start_error, AccountReason, HelperErrorCode, ACCOUNT_UNAVAILABLE,
};
use crate::share::exec_supervisor_protocol::RunAsAccount;

use super::exec_helper_client::HELPER_SOCKET;
use super::exec_helper_config;

/// Start buffer of the `*_r` lookups; doubled on `ERANGE` up to `MAX_BUFFER`.
const START_BUFFER: usize = 16 * 1024;
const MAX_BUFFER: usize = 1024 * 1024;
/// glibc's own bound for supplementary groups (`NGROUPS_MAX` is 65536).
const MAX_GROUPS: usize = 65_536;
const DEFAULT_SHELL: &str = "/bin/sh";
/// Groups whose members may become root: `sudo` (Debian, Ubuntu), `wheel`
/// (Fedora, RHEL) and `admin` (Ubuntu before 12.04).
const ADMIN_GROUPS: [&CStr; 3] = [c"sudo", c"wheel", c"admin"];
/// Login shells that run no command.
const NO_LOGIN_SHELLS: [&str; 2] = ["nologin", "false"];

pub(super) fn resolve(name: &str) -> io::Result<RunAsAccount> {
    let c_name = CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "account name contains NUL"))?;
    let mut buffer = vec![0u8; START_BUFFER];
    loop {
        // SAFETY: `passwd` is plain data; `getpwnam_r` fills it with pointers
        // into `buffer`, which outlives every use below.
        let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let status = unsafe {
            libc::getpwnam_r(
                c_name.as_ptr(),
                &mut entry,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if status == libc::ERANGE && buffer.len() < MAX_BUFFER {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status));
        }
        if result.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("local account {name:?} does not exist on this host"),
            ));
        }
        let home = text(entry.pw_dir).unwrap_or_else(|| "/".to_string());
        let shell = text(entry.pw_shell)
            .filter(|shell| shell.starts_with('/'))
            .unwrap_or_else(|| DEFAULT_SHELL.to_string());
        let groups = group_list(&c_name, entry.pw_gid)?;
        return Ok(RunAsAccount {
            name: name.to_string(),
            uid: entry.pw_uid,
            gid: entry.pw_gid,
            groups,
            home: if home.is_empty() { "/".into() } else { home },
            shell,
        });
    }
}

/// The account a start of this worker runs as, resolved at admission before
/// any process exists. A worker that is not root needs the account service
/// besides: its configuration allows `name` for this worker's uid and its
/// socket exists.
pub(super) fn resolve_for_start(name: &str) -> io::Result<RunAsAccount> {
    // SAFETY: `geteuid` only reads this process's id.
    let uid = unsafe { libc::geteuid() };
    if uid != 0 {
        let config = exec_helper_config::load().map_err(|error| {
            account_start_error(
                ACCOUNT_HELPER_UNAVAILABLE,
                format!("the account service configuration cannot be read: {error}"),
            )
        })?;
        let Some(config) = config else {
            return Err(account_start_error(
                ACCOUNT_HELPER_UNAVAILABLE,
                "the account service is not set up on this host",
            ));
        };
        if !config.allows(uid, name) {
            return Err(account_start_error(
                ACCOUNT_NOT_ALLOWED,
                format!("account {name} is not allowed for the remote commands of user {uid}"),
            ));
        }
        if std::fs::symlink_metadata(HELPER_SOCKET).is_err() {
            return Err(account_start_error(
                ACCOUNT_HELPER_UNAVAILABLE,
                format!("the account service does not listen at {HELPER_SOCKET}"),
            ));
        }
    }
    check_account(name).map_err(|refusal| refusal.into_error(name))
}

/// Why an account cannot run an account unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum AccountRefusal {
    InvalidName,
    NotFound,
    Lookup(String),
    Root,
    Expired,
    PasswordChangeRequired,
}

impl AccountRefusal {
    pub(super) fn detail(&self, name: &str) -> String {
        match self {
            Self::InvalidName => {
                format!("{name:?} is not a local account name an account unit can run as")
            }
            Self::NotFound => format!("account {name} does not exist on this host"),
            Self::Lookup(error) => {
                format!("account {name} could not be read from the user database: {error}")
            }
            Self::Root => "remote commands never run as root through another account; root \
                           runs only as the own account of a root worker"
                .into(),
            Self::Expired => format!("account {name} is expired or locked"),
            Self::PasswordChangeRequired => {
                format!("account {name} requires a password change; no remote access before that")
            }
        }
    }

    pub(super) fn reason(&self) -> AccountReason {
        match self {
            Self::InvalidName | Self::NotFound | Self::Lookup(_) => AccountReason::NotFound,
            Self::Root => AccountReason::Root,
            Self::Expired => AccountReason::Expired,
            Self::PasswordChangeRequired => AccountReason::PasswordChangeRequired,
        }
    }

    /// The account helper's error code (D.2.7).
    pub(super) fn helper_code(&self) -> HelperErrorCode {
        match self {
            Self::InvalidName | Self::NotFound | Self::Lookup(_) => HelperErrorCode::UnknownAccount,
            Self::Root => HelperErrorCode::NotAllowed,
            Self::Expired => HelperErrorCode::LogonFailed,
            Self::PasswordChangeRequired => HelperErrorCode::PasswordChangeRequired,
        }
    }

    /// The error a refused start carries to the admission.
    pub(super) fn into_error(self, name: &str) -> io::Error {
        account_start_error(ACCOUNT_UNAVAILABLE, self.detail(name))
    }
}

/// The checks every account unit passes before systemd starts it (PRM-M5).
pub(super) fn check_account(name: &str) -> Result<RunAsAccount, AccountRefusal> {
    if !unit_account_name(name) {
        return Err(AccountRefusal::InvalidName);
    }
    let account = resolve(name).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => AccountRefusal::NotFound,
        _ => AccountRefusal::Lookup(error.to_string()),
    })?;
    if account.uid == 0 {
        return Err(AccountRefusal::Root);
    }
    match shadow_state(name) {
        Ok(Some(ShadowState::Expired)) => Err(AccountRefusal::Expired),
        Ok(Some(ShadowState::PasswordChangeRequired)) => {
            Err(AccountRefusal::PasswordChangeRequired)
        }
        _ => Ok(account),
    }
}

/// `name` as `User=` of a unit: a name a grant may name, and not digits only,
/// which systemd would read as a uid.
pub(super) fn unit_account_name(name: &str) -> bool {
    valid_account_name(name) && !name.bytes().all(|byte| byte.is_ascii_digit())
}

/// What the shadow entry of an account says, as `pam_unix` reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ShadowState {
    Usable,
    Expired,
    PasswordChangeRequired,
}

/// The four day counts of a shadow entry that decide; -1 means unset.
#[derive(Clone, Copy)]
struct ShadowFields {
    last_change: libc::c_long,
    max: libc::c_long,
    inactive: libc::c_long,
    expire: libc::c_long,
}

/// `Ok(None)` without a shadow entry or without the right to read it: PAM
/// in the unit then decides alone.
pub(super) fn shadow_state(name: &str) -> io::Result<Option<ShadowState>> {
    let c_name = CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "account name contains NUL"))?;
    let mut buffer = vec![0u8; START_BUFFER];
    loop {
        // SAFETY: `spwd` is plain data; `getspnam_r` fills it with pointers
        // into `buffer`; only the numeric fields are read.
        let mut entry: libc::spwd = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::spwd = std::ptr::null_mut();
        let status = unsafe {
            libc::getspnam_r(
                c_name.as_ptr(),
                &mut entry,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if status == libc::ERANGE && buffer.len() < MAX_BUFFER {
            buffer.resize(buffer.len() * 2, 0);
            continue;
        }
        if matches!(status, libc::EACCES | libc::EPERM | libc::ENOENT)
            || (status == 0 && result.is_null())
        {
            return Ok(None);
        }
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status));
        }
        let fields = ShadowFields {
            last_change: entry.sp_lstchg,
            max: entry.sp_max,
            inactive: entry.sp_inact,
            expire: entry.sp_expire,
        };
        return Ok(Some(shadow_verdict(fields, today())));
    }
}

/// `check_shadow_expiry` of `pam_unix` (Linux-PAM `passverify.c`).
fn shadow_verdict(entry: ShadowFields, today: libc::c_long) -> ShadowState {
    let ShadowFields {
        last_change,
        max,
        inactive,
        expire,
    } = entry;
    if expire != -1 && today >= expire {
        return ShadowState::Expired;
    }
    if last_change == 0 {
        return ShadowState::PasswordChangeRequired;
    }
    if today < last_change {
        return ShadowState::Usable;
    }
    let since = today - last_change;
    if max != -1
        && inactive != -1
        && since > max
        && since > inactive
        && since > max.saturating_add(inactive)
    {
        return ShadowState::Expired;
    }
    if max != -1 && since > max {
        return ShadowState::PasswordChangeRequired;
    }
    ShadowState::Usable
}

fn today() -> libc::c_long {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    (seconds / 86_400) as libc::c_long
}

/// A warning when the account's login shell runs no command.
pub(super) fn shell_warning(account: &RunAsAccount) -> Option<String> {
    let shell = Path::new(&account.shell).file_name()?.to_str()?;
    NO_LOGIN_SHELLS.contains(&shell).then(|| {
        format!(
            "account {} has the login shell {}: shell commands and the terminal end at once; \
             programs started directly still run",
            account.name, account.shell
        )
    })
}

/// The account belongs to a group whose members may become root.
pub(super) fn admin_group(account: &RunAsAccount) -> bool {
    ADMIN_GROUPS
        .iter()
        .filter_map(|name| group_id(name))
        .any(|gid| account.groups.contains(&gid))
}

/// Accounts of people: uids from `UID_MIN` to `UID_MAX` of `/etc/login.defs`
/// (1000 and 60000 where it names none), read from `/etc/passwd`.
pub(super) fn login_accounts() -> Vec<String> {
    let definitions = std::fs::read_to_string("/etc/login.defs").unwrap_or_default();
    let first = login_definition(&definitions, "UID_MIN").unwrap_or(1000);
    let last = login_definition(&definitions, "UID_MAX").unwrap_or(60_000);
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    passwd
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            let uid: u32 = fields.nth(1)?.parse().ok()?;
            ((first..=last).contains(&uid) && unit_account_name(name)).then(|| name.to_string())
        })
        .collect()
}

fn login_definition(text: &str, key: &str) -> Option<u32> {
    text.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        if words.next()? != key {
            return None;
        }
        words.next()?.parse().ok()
    })
}

/// The account this process runs as; its uid where the user database has no
/// name for it.
pub(super) fn own_account_name() -> String {
    // SAFETY: `passwd` is plain data; `getpwuid_r` fills it with pointers into
    // `buffer`, which outlives the read of the name.
    unsafe {
        let uid = libc::geteuid();
        let mut pwd: libc::passwd = std::mem::zeroed();
        let mut result = std::ptr::null_mut();
        let mut buffer = vec![0u8; 4096];
        if libc::getpwuid_r(
            uid,
            &mut pwd,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        ) == 0
            && !result.is_null()
            && !pwd.pw_name.is_null()
        {
            return CStr::from_ptr(pwd.pw_name).to_string_lossy().into_owned();
        }
        uid.to_string()
    }
}

fn text(pointer: *const libc::c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    // SAFETY: a non-null field of a filled `passwd` is a NUL-terminated string.
    let value = unsafe { CStr::from_ptr(pointer) };
    Some(value.to_string_lossy().into_owned())
}

fn group_id(name: &CStr) -> Option<u32> {
    let mut buffer = vec![0u8; START_BUFFER];
    // SAFETY: `group` is plain data; `getgrnam_r` fills it with pointers into
    // `buffer`, of which only the numeric id is read.
    let mut group: libc::group = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::group = std::ptr::null_mut();
    let status = unsafe {
        libc::getgrnam_r(
            name.as_ptr(),
            &mut group,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    (status == 0 && !result.is_null()).then_some(group.gr_gid)
}

/// The primary group and every supplementary group of the account.
fn group_list(name: &CStr, gid: libc::gid_t) -> io::Result<Vec<u32>> {
    let mut capacity: usize = 64;
    loop {
        let mut groups = vec![0 as libc::gid_t; capacity];
        let mut count = capacity as libc::c_int;
        // SAFETY: `groups` holds `count` entries; glibc writes at most that
        // many and reports the needed number in `count` when it is too small.
        let status =
            unsafe { libc::getgrouplist(name.as_ptr(), gid, groups.as_mut_ptr(), &mut count) };
        let needed = usize::try_from(count).unwrap_or(0);
        if status >= 0 {
            groups.truncate(needed.min(capacity));
            if !groups.contains(&gid) {
                groups.insert(0, gid);
            }
            return Ok(groups);
        }
        if needed <= capacity || needed > MAX_GROUPS {
            return Err(io::Error::other(
                "the group list of the run-as account could not be read",
            ));
        }
        capacity = needed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luecken_task_root_and_missing_accounts_resolve_as_the_user_database_says() {
        let root = resolve("root").unwrap();
        assert_eq!(root.uid, 0);
        assert_eq!(root.gid, 0);
        assert!(root.groups.contains(&0));
        assert!(root.home.starts_with('/'));
        assert!(root.shell.starts_with('/'));
        let missing = resolve("se-no-such-account-4711").unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn linux_account_checks_follow_pam_unix_and_systemd_names() {
        use ShadowState::{Expired, PasswordChangeRequired as Change, Usable};
        for (last_change, max, inactive, expire, verdict) in [
            (19_000, 99_999, -1, -1, Usable),
            (19_000, -1, -1, 0, Expired),
            (0, 99_999, -1, -1, Change),
            (19_000, 90, -1, -1, Change),
            (19_000, 90, 30, -1, Expired),
            (-1, -1, -1, -1, Usable),
        ] {
            let entry = ShadowFields {
                last_change,
                max,
                inactive,
                expire,
            };
            assert_eq!(
                shadow_verdict(entry, 20_000),
                verdict,
                "{last_change} {max}"
            );
        }
        assert!(unit_account_name("se-remote") && !unit_account_name("1000"));
        assert!(!unit_account_name("-x"));
        assert_eq!(check_account("root").unwrap_err(), AccountRefusal::Root);
        let missing = check_account("se-no-such-account-4711").unwrap_err();
        assert_eq!(missing, AccountRefusal::NotFound);
        let definitions = "# comment\nUID_MIN\t\t 1000\nUID_MAX 60000\n";
        assert_eq!(login_definition(definitions, "UID_MIN"), Some(1000));
        assert_eq!(login_definition(definitions, "GID_MIN"), None);
    }
}
