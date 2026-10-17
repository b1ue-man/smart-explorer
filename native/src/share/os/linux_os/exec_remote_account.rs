//! Accounts for remote commands. The restricted remote-access account
//! `se-remote` (C3, PRM-M5 (c)) is a system account with its own group, its
//! home in `/var/lib/smart-explorer/se-remote`, `bash` (else `sh`) as login
//! shell, a locked password and no password ageing, so `pam_acct_mgmt`
//! accepts it (NR N20). Creating it is the explicit "Fernzugriffskonto
//! anlegen"; only an explicit request deletes it, after its units ended.
//! The account choices ("Ausführen als", D.2.4) list it with every other
//! account this worker may use.
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use zbus::blocking::Connection;
use zbus::zvariant::OwnedObjectPath;

use crate::share::access_catalog::{HelperState, HelperStatus};
use crate::share::access_grant::REMOTE_ACCOUNT_NAME;
use crate::share::exec_helper_protocol::{
    AccountChoice, AccountChoiceProfile, AccountChoices, AccountReason,
};

use super::exec_account::{self, AccountRefusal};
use super::exec_account_unit::UNIT_PREFIX;
use super::exec_binary_copy::ensure_root_directory;
use super::exec_helper_client::{self, HELPER_SOCKET};
use super::exec_helper_config;
use super::exec_systemd::{self, eio, manager_connection, require_cgroup_v2};

const HOME_PARENT: &str = "/var/lib/smart-explorer";
const REMOTE_ACCOUNT_HOME: &str = "/var/lib/smart-explorer/se-remote";
/// Search path of the account tools; the caller's `PATH` is not used.
const SYSTEM_PATH: &str = "/usr/sbin:/usr/bin:/sbin:/bin";
/// `useradd`: the account exists already.
const USERADD_EXISTS: i32 = 9;
/// `userdel`: the account does not exist.
const USERDEL_MISSING: i32 = 6;
const UNIT_STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// Creates `se-remote` (root only); `Ok(false)` when it exists already.
pub(super) fn ensure_remote_account() -> io::Result<bool> {
    require_root()?;
    if let Ok(account) = exec_account::resolve(REMOTE_ACCOUNT_NAME) {
        if account.uid == 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the account se-remote has uid 0 on this host; Smart Explorer does not use it",
            ));
        }
        return Ok(false);
    }
    ensure_root_directory(Path::new(HOME_PARENT))?;
    let shell = if Path::new("/bin/bash").is_file() {
        "/bin/bash"
    } else {
        "/bin/sh"
    };
    let output = account_tool("useradd")?
        .args([
            "--system",
            "--create-home",
            "--home-dir",
            REMOTE_ACCOUNT_HOME,
            "--shell",
            shell,
            "--user-group",
            "--comment",
            "Smart Explorer remote access",
            REMOTE_ACCOUNT_NAME,
        ])
        .output()?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(USERADD_EXISTS) => Ok(false),
        _ => Err(io::Error::other(format!(
            "useradd could not create se-remote: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

/// Stops the units of `se-remote`, then deletes it with its home (root
/// only); `Ok(false)` when there was none.
pub(super) fn delete_remote_account() -> io::Result<bool> {
    require_root()?;
    if exec_account::resolve(REMOTE_ACCOUNT_NAME).is_err() {
        return Ok(false);
    }
    stop_remote_units()?;
    let output = account_tool("userdel")?
        .args(["--remove", REMOTE_ACCOUNT_NAME])
        .output()?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(USERDEL_MISSING) => Ok(false),
        _ => Err(io::Error::other(format!(
            "userdel could not delete se-remote: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
    }
}

/// "Ausführen als" of this host (D.2.4): the own account; `se-remote`; for a
/// root worker every account of a person, otherwise the accounts the account
/// service allows this worker. Each with availability, reason and next step.
pub(super) fn account_choices() -> AccountChoices {
    let own = exec_account::own_account_name();
    // SAFETY: `geteuid` only reads this process's id.
    let uid = unsafe { libc::geteuid() };
    let root = uid == 0;
    let (helper, allowed) = if root {
        (root_helper_status(), exec_account::login_accounts())
    } else {
        let config = exec_helper_config::load().ok().flatten();
        let allowed = config.map(|config| config.accounts_for(uid).to_vec());
        (
            exec_helper_client::helper_status(),
            allowed.unwrap_or_default(),
        )
    };
    let mut choices = vec![AccountChoice::own(&own)];
    let mut remote = AccountChoice::other(AccountChoiceProfile::Remote, REMOTE_ACCOUNT_NAME);
    assess(&mut remote, root, &helper, &allowed);
    choices.push(remote);
    for name in &allowed {
        if name != REMOTE_ACCOUNT_NAME && *name != own {
            let mut choice = AccountChoice::other(AccountChoiceProfile::Account, name);
            assess(&mut choice, root, &helper, &allowed);
            choices.push(choice);
        }
    }
    AccountChoices { helper, choices }
}

/// Whether a grant may name another account: a root worker, or a worker the
/// configuration of the account service lists while its socket exists. The
/// same facts as `account_choices`, without asking the service.
pub(super) fn run_as_supported() -> bool {
    // SAFETY: `geteuid` only reads this process's id.
    let uid = unsafe { libc::geteuid() };
    uid == 0
        || (std::fs::symlink_metadata(HELPER_SOCKET).is_ok()
            && exec_helper_config::load()
                .ok()
                .flatten()
                .is_some_and(|config| config.has_user(uid)))
}

fn root_helper_status() -> HelperStatus {
    let detail = require_cgroup_v2()
        .and_then(|()| manager_connection().map(drop))
        .err()
        .map(|error| error.to_string());
    HelperStatus {
        state: if detail.is_none() {
            HelperState::NotNeeded
        } else {
            HelperState::Unavailable
        },
        version: None,
        detail,
    }
}

/// Availability, reason and next step of one other account.
fn assess(choice: &mut AccountChoice, root: bool, helper: &HelperStatus, allowed: &[String]) {
    let name = choice.label.clone();
    let detail = helper.detail.clone().unwrap_or_default();
    let setup = "as administrator: se share exec accounts setup --remote-account";
    match helper.state {
        HelperState::Unavailable => choice.refuse(AccountReason::Unsupported, detail, None),
        HelperState::NotInstalled => choice.refuse(
            AccountReason::HelperNotInstalled,
            "the account service is not set up on this host".into(),
            Some(setup.into()),
        ),
        HelperState::Outdated => choice.refuse(
            AccountReason::HelperOutdated,
            format!("the account service needs an update ({detail})"),
            Some("as administrator: se share exec accounts update".into()),
        ),
        HelperState::NotNeeded | HelperState::Installed if !root && !allowed.contains(&name) => {
            choice.refuse(
                AccountReason::NotAllowed,
                format!("account {name} is not allowed for the remote commands of this user"),
                Some(format!(
                    "as administrator: se share exec accounts allow {name}"
                )),
            )
        }
        HelperState::NotNeeded | HelperState::Installed => {
            match exec_account::check_account(&name) {
                Err(refusal) => {
                    let missing_remote =
                        refusal == AccountRefusal::NotFound && name == REMOTE_ACCOUNT_NAME;
                    let next_step = missing_remote.then(|| setup.to_string());
                    choice.refuse(refusal.reason(), refusal.detail(&name), next_step);
                }
                Ok(account) => {
                    choice.admin_group = Some(exec_account::admin_group(&account));
                    if let Some(warning) = exec_account::shell_warning(&account) {
                        choice.warn(AccountReason::NoLoginShell, warning);
                    }
                }
            }
        }
    }
}

/// Stops every job unit that runs as `se-remote` and waits until each has
/// ended, so `userdel` finds no process of it.
fn stop_remote_units() -> io::Result<()> {
    let manager = Connection::system().map_err(eio)?;
    let mut stopped = Vec::new();
    for name in exec_systemd::unit_names(&manager, &format!("{UNIT_PREFIX}*.service"))? {
        let Some(path) = exec_systemd::loaded_unit(&manager, &name)? else {
            continue;
        };
        if exec_systemd::unit_user(&manager, &path)? == REMOTE_ACCOUNT_NAME {
            exec_systemd::stop_unit(&manager, &name)?;
            stopped.push((name, path));
        }
    }
    let deadline = Instant::now() + UNIT_STOP_TIMEOUT;
    for (name, path) in &stopped {
        while Instant::now() < deadline && unit_running(&manager, name, path)? {
            std::thread::sleep(Duration::from_millis(50));
        }
        exec_systemd::reset_failed_unit(&manager, name)?;
    }
    Ok(())
}

fn unit_running(manager: &Connection, name: &str, path: &OwnedObjectPath) -> io::Result<bool> {
    if exec_systemd::loaded_unit(manager, name)?.is_none() {
        return Ok(false);
    }
    let state =
        exec_systemd::unit_active_state(manager, path).unwrap_or_else(|_| "inactive".into());
    Ok(!matches!(state.as_str(), "inactive" | "failed"))
}

/// The account tool `name` from `/usr/sbin` or `/sbin`, with a fixed search
/// path and no input.
fn account_tool(name: &str) -> io::Result<Command> {
    let program: PathBuf = ["/usr/sbin", "/sbin"]
        .iter()
        .map(|directory| Path::new(directory).join(name))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("{name} is not installed in /usr/sbin or /sbin"),
            )
        })?;
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", SYSTEM_PATH)
        .stdin(Stdio::null());
    Ok(command)
}

fn require_root() -> io::Result<()> {
    // SAFETY: `geteuid` only reads this process's id.
    if unsafe { libc::geteuid() } == 0 {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "creating or deleting se-remote needs administrator rights",
        ))
    }
}
