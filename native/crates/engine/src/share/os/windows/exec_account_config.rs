//! Configuration of the Windows account service (contract D.2.8):
//! `%ProgramData%\Smart Explorer\exec-accounts.json` lists per Windows user
//! (SID) the local accounts that user's worker may run remote commands as:
//!
//! ```json
//! {"version":1,"users":[{"sid":"S-1-5-21-…","accounts":["se-remote","deploy"]}]}
//! ```
//!
//! SYSTEM and the Administrators change the file, users read it
//! (`exec_account_files.rs` checks its owner before trusting it). A file of
//! another version, an invalid SID, an invalid or administrator account name
//! or a user listed twice makes the whole file invalid; the service then
//! refuses every request. The names of the service, its pipe and its files
//! are here too.
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use windows_sys::core::GUID;
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::UI::Shell::{
    FOLDERID_ProgramData, FOLDERID_ProgramFiles, SHGetKnownFolderPath, KF_FLAG_DEFAULT,
};

use crate::share::access_grant::valid_account_name;

use super::exec_account_files::read_trusted;
use super::exec_account_security::{from_wide_ptr, sid_text};

pub(super) const SERVICE_NAME: &str = "SmartExplorerExecAccounts";
pub(super) const SERVICE_DISPLAY_NAME: &str = "Smart Explorer Kontodienst";
/// The service's pipe; only SYSTEM and the registered users may open it.
pub(super) const SERVICE_PIPE: &str = r"\\.\pipe\SmartExplorerExecAccounts";
/// Version of the configuration file.
const CONFIG_VERSION: u32 = 1;
const MAX_CONFIG_BYTES: u64 = 64 * 1024;
/// Accounts the service never runs as, whatever a file lists (C2): the
/// built-in administrator and the system accounts by their names; the logon
/// itself refuses every other administrator.
const NEVER: [&str; 6] = [
    "administrator",
    "system",
    "localsystem",
    "local service",
    "network service",
    "guest",
];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ServiceConfig {
    pub(super) version: u32,
    #[serde(default)]
    pub(super) users: Vec<UserAccounts>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct UserAccounts {
    pub(super) sid: String,
    #[serde(default)]
    pub(super) accounts: Vec<String>,
}

impl ServiceConfig {
    pub(super) fn new() -> Self {
        Self {
            version: CONFIG_VERSION,
            users: Vec::new(),
        }
    }

    pub(super) fn parse(bytes: &[u8]) -> Result<Self, String> {
        let config: Self =
            serde_json::from_slice(bytes).map_err(|error| format!("not valid JSON: {error}"))?;
        if config.version != CONFIG_VERSION {
            return Err(format!(
                "version {} is not {CONFIG_VERSION}",
                config.version
            ));
        }
        for (index, user) in config.users.iter().enumerate() {
            if !sid_text(&user.sid) {
                return Err(format!("{:?} is not a SID", user.sid));
            }
            if config.users[..index]
                .iter()
                .any(|other| other.sid.eq_ignore_ascii_case(&user.sid))
            {
                return Err(format!("{} is listed twice", user.sid));
            }
            if let Some(account) = user.accounts.iter().find(|name| !allowable_name(name)) {
                return Err(format!("{account:?} cannot be allowed for remote commands"));
            }
        }
        Ok(config)
    }

    pub(super) fn render(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_vec_pretty(self).unwrap_or_default();
        bytes.push(b'\n');
        bytes
    }

    fn user(&self, sid: &str) -> Option<&UserAccounts> {
        self.users
            .iter()
            .find(|user| user.sid.eq_ignore_ascii_case(sid))
    }

    pub(super) fn has_user(&self, sid: &str) -> bool {
        self.user(sid).is_some()
    }

    /// The accounts `sid` may use; none for a user the file does not list.
    pub(super) fn accounts_for(&self, sid: &str) -> &[String] {
        self.user(sid)
            .map(|user| user.accounts.as_slice())
            .unwrap_or_default()
    }

    /// Windows account names do not depend on case.
    pub(super) fn allows(&self, sid: &str, account: &str) -> bool {
        self.accounts_for(sid)
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(account))
    }

    pub(super) fn user_sids(&self) -> Vec<&str> {
        self.users.iter().map(|user| user.sid.as_str()).collect()
    }

    /// Lists `sid` without accounts where it was missing.
    pub(super) fn add_user(&mut self, sid: &str) -> Result<bool, String> {
        if !sid_text(sid) {
            return Err(format!("{sid:?} is not a SID"));
        }
        if self.has_user(sid) {
            return Ok(false);
        }
        self.users.push(UserAccounts {
            sid: sid.to_string(),
            accounts: Vec::new(),
        });
        Ok(true)
    }

    /// Allows `account` for `sid`; `Ok(false)` when it was allowed already.
    pub(super) fn allow(&mut self, sid: &str, account: &str) -> Result<bool, String> {
        if !allowable_name(account) {
            return Err(format!("{account:?} cannot be allowed for remote commands"));
        }
        self.add_user(sid)?;
        if self.allows(sid, account) {
            return Ok(false);
        }
        let user = self
            .users
            .iter_mut()
            .find(|user| user.sid.eq_ignore_ascii_case(sid))
            .ok_or_else(|| format!("{sid} is missing"))?;
        user.accounts.push(account.to_string());
        Ok(true)
    }

    /// Takes `account` away from `sid`; `false` when it was not allowed.
    pub(super) fn disallow(&mut self, sid: &str, account: &str) -> bool {
        let Some(user) = self
            .users
            .iter_mut()
            .find(|user| user.sid.eq_ignore_ascii_case(sid))
        else {
            return false;
        };
        let before = user.accounts.len();
        user.accounts
            .retain(|allowed| !allowed.eq_ignore_ascii_case(account));
        user.accounts.len() != before
    }
}

/// The configuration; `Ok(None)` when there is none. A file that is not
/// trusted (`exec_account_files.rs`) or not valid is an error.
pub(super) fn load_config() -> io::Result<Option<ServiceConfig>> {
    let Some(bytes) = read_trusted(&config_path()?, MAX_CONFIG_BYTES)? else {
        return Ok(None);
    };
    ServiceConfig::parse(&bytes)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// A name the configuration may list: a valid account name of a grant and
/// none of the accounts the service never runs as.
pub(super) fn allowable_name(name: &str) -> bool {
    valid_account_name(name) && !NEVER.contains(&name.to_ascii_lowercase().as_str())
}

/// `%ProgramData%\Smart Explorer`, the folder of configuration, log and
/// setup reports.
pub(super) fn data_dir() -> io::Result<PathBuf> {
    Ok(known_folder(&FOLDERID_ProgramData)?.join("Smart Explorer"))
}

pub(super) fn config_path() -> io::Result<PathBuf> {
    Ok(data_dir()?.join("exec-accounts.json"))
}

/// The service's log (C5): SYSTEM and the Administrators only.
pub(super) fn log_path() -> io::Result<PathBuf> {
    Ok(data_dir()?.join("exec-accounts.log"))
}

/// The report of an elevated setup for the user `sid`, who may read it.
pub(super) fn report_path(sid: &str) -> io::Result<PathBuf> {
    Ok(data_dir()?.join(format!("exec-accounts-report-{sid}.txt")))
}

/// `%ProgramFiles%\Smart Explorer\exec-accounts`: the folder of the program
/// copy, which only administrators change (the installation of the app is
/// the user's own and writable for the user, B8).
pub(super) fn program_dir() -> io::Result<PathBuf> {
    Ok(known_folder(&FOLDERID_ProgramFiles)?
        .join("Smart Explorer")
        .join("exec-accounts"))
}

/// The program copy the service and its relays run.
pub(super) fn program_copy() -> io::Result<PathBuf> {
    Ok(program_dir()?.join("se.exe"))
}

/// A known folder from the system's registration, not from the
/// environment of the calling process.
fn known_folder(id: &GUID) -> io::Result<PathBuf> {
    let mut path: *mut u16 = std::ptr::null_mut();
    let result = unsafe {
        SHGetKnownFolderPath(id, KF_FLAG_DEFAULT as u32, std::ptr::null_mut(), &mut path)
    };
    // SAFETY: on success `path` is a NUL-terminated string the shell
    // allocated; on failure it is null or allocated as well.
    let text = unsafe { from_wide_ptr(path) };
    unsafe { CoTaskMemFree(path as *const std::ffi::c_void) };
    if result < 0 || text.is_empty() {
        return Err(io::Error::other(format!(
            "Windows names no known folder (HRESULT {result:#010x})"
        )));
    }
    Ok(PathBuf::from(text))
}
