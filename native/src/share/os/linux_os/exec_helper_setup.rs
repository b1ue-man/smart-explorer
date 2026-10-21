//! Setting up the account service (PRM-M6, D.5): what `se share exec
//! accounts setup|allow|disallow|update|remove` does as administrator. It
//! installs the program copy (`exec_binary_copy`), the PAM service file
//! (`exec_pam_service`), on request the account `se-remote`
//! (`exec_remote_account`), the configuration (`exec_helper_config`) and
//! the socket and service units, which the system manager enables. Without
//! a user to act for (root itself) it installs only what a root worker's
//! account units need: program copy, PAM service file, `se-remote`. A worker
//! without administrator rights asks for them through `pkexec`, which runs
//! this program's hidden mode `--exec-accounts-setup` or
//! `--exec-accounts-remove`; where `pkexec` is missing or nobody confirms
//! it, the worker gets the `sudo` command a person runs instead.
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use zbus::blocking::Connection;

use crate::share::access_grant::REMOTE_ACCOUNT_NAME;

use super::exec_account::{self, AccountRefusal};
use super::exec_binary_copy::{self, PROGRAM_COPY};
use super::exec_helper_client::HELPER_SOCKET;
use super::exec_helper_config::{self, CONFIG_FILE};
use super::exec_helper_server::{service_unit_text, socket_unit_text};
use super::exec_pam_service::{self, PamFileState, PAM_FILE};
use super::exec_remote_account;
use super::exec_systemd::{self, eio};

pub(super) const SOCKET_UNIT: &str = "smart-explorer-exec-helper.socket";
pub(super) const SOCKET_UNIT_FILE: &str = "/etc/systemd/system/smart-explorer-exec-helper.socket";
pub(super) const SERVICE_UNIT_FILE: &str =
    "/etc/systemd/system/smart-explorer-exec-helper@.service";
/// The service's instances, one per connection.
const SERVICE_INSTANCES: &str = "smart-explorer-exec-helper@*.service";
const SETUP_MODE: &str = "--exec-accounts-setup";
const REMOVE_MODE: &str = "--exec-accounts-remove";
const PKEXEC: [&str; 2] = ["/usr/bin/pkexec", "/bin/pkexec"];
/// `pkexec`: the person dismissed the dialog (126) or nobody authorized (127).
const PKEXEC_DISMISSED: i32 = 126;
const PKEXEC_REFUSED: i32 = 127;

/// What a person asks for (D.2.4, `se share exec accounts …`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExecAccountsAction {
    /// Installs or renews the account service and allows the invoking user
    /// `se-remote` (with `remote_account`, which also creates it) and `allow`.
    Setup {
        remote_account: bool,
        allow: Vec<String>,
    },
    Allow {
        account: String,
    },
    Disallow {
        account: String,
    },
    /// Renews program copy, PAM service file and units after an update.
    Update,
    /// Removes the account service; `se-remote` only on explicit request.
    Remove {
        delete_remote_account: bool,
    },
}

/// How a worker without administrator rights asks for them.
#[allow(dead_code)] // `se share exec accounts` and the desktop accounts page pick one
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Elevation {
    /// Desktop: a graphical polkit agent asks; the output is collected.
    PkexecGraphical,
    /// CLI on a terminal: `pkexec` asks there, and the output goes there.
    PkexecTerminal,
    /// Never start `pkexec`; return the `sudo` command.
    SudoCommandOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExecAccountsOutcome {
    /// Done: one line per step (none where `pkexec` printed them already).
    Done(Vec<String>),
    /// Administrator rights are missing and could not be asked for here:
    /// the command a person runs, and why.
    NeedsAdministrator { command: String, reason: String },
}

/// Runs `action` as administrator: directly in a root process (for the
/// user who ran `sudo` or `pkexec`), otherwise through `pkexec` as
/// `elevation` allows.
#[allow(dead_code)] // `se share exec accounts` and the desktop accounts page
pub(crate) fn exec_accounts_apply(
    action: &ExecAccountsAction,
    elevation: Elevation,
) -> io::Result<ExecAccountsOutcome> {
    if is_root() {
        return apply(action, invoking_uid()).map(ExecAccountsOutcome::Done);
    }
    let program = elevation_program()?;
    let command = sudo_command(&program, action);
    let needs = |reason: &str| ExecAccountsOutcome::NeedsAdministrator {
        command: command.clone(),
        reason: reason.into(),
    };
    let pkexec = match (elevation, pkexec_program()) {
        (Elevation::SudoCommandOnly, _) => return Ok(needs("administrator rights are needed")),
        (_, None) => return Ok(needs("pkexec is not installed")),
        (_, Some(pkexec)) => pkexec,
    };
    // SAFETY: `getuid` only reads this process's id.
    let uid = unsafe { libc::getuid() };
    let mut process = Command::new(pkexec);
    process.arg(&program).args(mode_arguments(action, uid));
    let (code, output, errors) = if elevation == Elevation::PkexecTerminal {
        (process.status()?.code(), String::new(), String::new())
    } else {
        let output = process.stdin(Stdio::null()).output()?;
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    match code {
        Some(0) => Ok(ExecAccountsOutcome::Done(
            output.lines().map(str::to_string).collect(),
        )),
        Some(PKEXEC_DISMISSED | PKEXEC_REFUSED) => {
            Ok(needs("nobody confirmed the administrator request (pkexec)"))
        }
        _ => Err(io::Error::other(format!(
            "setting up the account service failed: {}",
            errors.trim()
        ))),
    }
}

/// The Smart Explorer user an administrator acts for: who ran `pkexec` or
/// `sudo` (`PKEXEC_UID`, `SUDO_UID`); `None` for root itself.
pub(crate) fn invoking_uid() -> Option<u32> {
    ["PKEXEC_UID", "SUDO_UID"]
        .iter()
        .find_map(|name| std::env::var(name).ok()?.parse::<u32>().ok())
        .filter(|uid| *uid != 0)
}

/// The hidden modes `pkexec` runs: `--exec-accounts-setup --for-uid UID`
/// with the words of `se share exec accounts` (D.2.4), and
/// `--exec-accounts-remove --for-uid UID [--delete-remote-account]`. Each
/// step's line goes to standard output.
pub(super) fn run_if_requested(arguments: &[OsString]) -> Option<io::Result<()>> {
    let first = arguments.first()?;
    if first != SETUP_MODE && first != REMOVE_MODE {
        return None;
    }
    let result = parse_mode(arguments)
        .and_then(|(action, uid)| apply(&action, Some(uid)))
        .map(|lines| lines.iter().for_each(|line| println!("{line}")));
    Some(result)
}

fn parse_mode(arguments: &[OsString]) -> io::Result<(ExecAccountsAction, u32)> {
    let words = arguments
        .iter()
        .map(|word| {
            word.to_str()
                .ok_or_else(|| invalid("an argument is not UTF-8"))
        })
        .collect::<io::Result<Vec<&str>>>()?;
    let [mode, "--for-uid", uid, rest @ ..] = words.as_slice() else {
        return Err(invalid("the setup mode needs --for-uid"));
    };
    let uid = uid.parse::<u32>().map_err(|_| invalid("invalid uid"))?;
    let action = if *mode == REMOVE_MODE {
        parse_words(&[&["remove"][..], rest].concat())?
    } else {
        parse_words(rest)?
    };
    Ok((action, uid))
}

/// `action` in the words of `se share exec accounts` (D.2.4).
fn cli_words(action: &ExecAccountsAction) -> Vec<String> {
    let mut words = Vec::new();
    match action {
        ExecAccountsAction::Setup {
            remote_account,
            allow,
        } => {
            words.push("setup".to_string());
            if *remote_account {
                words.push("--remote-account".into());
            }
            for account in allow {
                words.extend(["--allow".to_string(), account.clone()]);
            }
            words.push("--yes".into());
        }
        ExecAccountsAction::Allow { account } => {
            words.extend(["allow".to_string(), account.clone()]);
        }
        ExecAccountsAction::Disallow { account } => {
            words.extend(["disallow".to_string(), account.clone()]);
        }
        ExecAccountsAction::Update => words.push("update".into()),
        ExecAccountsAction::Remove {
            delete_remote_account,
        } => {
            words.push("remove".into());
            if *delete_remote_account {
                words.push("--delete-remote-account".into());
            }
        }
    }
    words
}

fn parse_words(words: &[&str]) -> io::Result<ExecAccountsAction> {
    let action = match words {
        ["setup", options @ ..] => {
            let (mut remote_account, mut allow, mut rest) = (false, Vec::new(), options);
            loop {
                match rest {
                    ["--remote-account", tail @ ..] => (remote_account, rest) = (true, tail),
                    ["--allow", account, tail @ ..] => {
                        allow.push((*account).to_string());
                        rest = tail;
                    }
                    ["--yes"] | [] => break,
                    _ => return Err(invalid("unknown option of setup")),
                }
            }
            ExecAccountsAction::Setup {
                remote_account,
                allow,
            }
        }
        ["allow", account] => ExecAccountsAction::Allow {
            account: (*account).into(),
        },
        ["disallow", account] => ExecAccountsAction::Disallow {
            account: (*account).into(),
        },
        ["update"] => ExecAccountsAction::Update,
        ["remove"] => ExecAccountsAction::Remove {
            delete_remote_account: false,
        },
        ["remove", "--delete-remote-account"] => ExecAccountsAction::Remove {
            delete_remote_account: true,
        },
        _ => return Err(invalid("unknown operation or options of the setup mode")),
    };
    Ok(action)
}

fn mode_arguments(action: &ExecAccountsAction, uid: u32) -> Vec<String> {
    let mut words = cli_words(action);
    let mode = if matches!(action, ExecAccountsAction::Remove { .. }) {
        words.remove(0);
        REMOVE_MODE
    } else {
        SETUP_MODE
    };
    [mode.to_string(), "--for-uid".into(), uid.to_string()]
        .into_iter()
        .chain(words)
        .collect()
}

/// The command a person runs with `sudo`, in the syntax of D.2.4.
fn sudo_command(program: &Path, action: &ExecAccountsAction) -> String {
    let words: Vec<String> = cli_words(action)
        .iter()
        .map(|word| shell_word(word))
        .collect();
    format!(
        "sudo {} share exec accounts {}",
        shell_word(&program.to_string_lossy()),
        words.join(" ")
    )
}

fn shell_word(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"/._-+=:@".contains(&byte));
    if plain {
        word.into()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// `se` beside the running program where it exists (the desktop app has
/// it next to itself), else the running program.
fn elevation_program() -> io::Result<PathBuf> {
    let running = std::fs::read_link("/proc/self/exe")?;
    let beside = running.with_file_name("se");
    Ok(if beside.is_file() { beside } else { running })
}

fn pkexec_program() -> Option<&'static str> {
    PKEXEC.into_iter().find(|path| Path::new(path).is_file())
}

fn apply(action: &ExecAccountsAction, for_uid: Option<u32>) -> io::Result<Vec<String>> {
    if !is_root() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the account service is set up by an administrator only",
        ));
    }
    match action {
        ExecAccountsAction::Setup {
            remote_account,
            allow,
        } => setup(for_uid, *remote_account, allow),
        ExecAccountsAction::Allow { account } => change(for_uid, account, true),
        ExecAccountsAction::Disallow { account } => change(for_uid, account, false),
        ExecAccountsAction::Update => update(),
        ExecAccountsAction::Remove {
            delete_remote_account,
        } => remove(*delete_remote_account),
    }
}

fn setup(for_uid: Option<u32>, remote_account: bool, allow: &[String]) -> io::Result<Vec<String>> {
    for account in allow {
        allowable(account)?;
    }
    exec_systemd::require_cgroup_v2()?;
    let mut done = install_files()?;
    if remote_account {
        done.push(if exec_remote_account::ensure_remote_account()? {
            "created the remote access account se-remote".into()
        } else {
            "the remote access account se-remote exists".into()
        });
    }
    let Some(uid) = for_uid else {
        if !allow.is_empty() {
            return Err(invalid(
                "allowing accounts needs the user they are for (sudo, pkexec)",
            ));
        }
        done.push("no user to register: the account units of a root worker need no service".into());
        return Ok(done);
    };
    let mut config = exec_helper_config::load()?.unwrap_or_default();
    config.add_user(uid).map_err(invalid)?;
    let remote = remote_account.then_some(REMOTE_ACCOUNT_NAME);
    for account in remote.into_iter().chain(allow.iter().map(String::as_str)) {
        config.allow(uid, account).map_err(invalid)?;
    }
    exec_helper_config::store(&config)?;
    done.push(format!(
        "user {uid} may run remote commands as: {}",
        config.accounts_for(uid).join(", ")
    ));
    done.extend(install_units()?);
    Ok(done)
}

fn change(for_uid: Option<u32>, account: &str, allow: bool) -> io::Result<Vec<String>> {
    let uid = for_uid.ok_or_else(|| invalid("changing the list needs its user (sudo, pkexec)"))?;
    let mut config = exec_helper_config::load()?.ok_or_else(not_set_up)?;
    let changed = if allow {
        allowable(account)?;
        config.allow(uid, account).map_err(invalid)?
    } else {
        config.disallow(uid, account)
    };
    exec_helper_config::store(&config)?;
    Ok(vec![match (allow, changed) {
        (true, true) => format!("user {uid} may now run remote commands as {account}"),
        (true, false) => format!("user {uid} could run remote commands as {account} already"),
        (false, true) => format!("user {uid} may no longer run remote commands as {account}"),
        (false, false) => format!("{account} was not allowed for user {uid}"),
    }])
}

fn update() -> io::Result<Vec<String>> {
    if std::fs::symlink_metadata(SOCKET_UNIT_FILE).is_err() {
        return Err(not_set_up());
    }
    let mut done = install_files()?;
    done.extend(install_units()?);
    Ok(done)
}

/// The program copy and the PAM service file, which every account unit
/// needs.
fn install_files() -> io::Result<Vec<String>> {
    let hash = exec_binary_copy::install_copy()?;
    let pam = match exec_pam_service::ensure_installed()? {
        PamFileState::Created => format!("created {PAM_FILE}"),
        PamFileState::Updated => format!("updated {PAM_FILE}"),
        PamFileState::Current => format!("{PAM_FILE} is current"),
        PamFileState::Foreign => format!("kept {PAM_FILE}, which an administrator manages"),
    };
    Ok(vec![
        format!("installed {PROGRAM_COPY} (SHA-256 {hash})"),
        pam,
    ])
}

/// Writes both unit files, reloads, enables and restarts the listening socket.
fn install_units() -> io::Result<Vec<String>> {
    exec_binary_copy::write_root_file(
        Path::new(SOCKET_UNIT_FILE),
        socket_unit_text().as_bytes(),
        0o644,
    )?;
    exec_binary_copy::write_root_file(
        Path::new(SERVICE_UNIT_FILE),
        service_unit_text().as_bytes(),
        0o644,
    )?;
    let manager = manager()?;
    exec_systemd::reload_manager(&manager)?;
    exec_systemd::enable_unit_file(&manager, SOCKET_UNIT)?;
    socket_restart::restart_listening(&manager)?;
    Ok(vec![
        format!("wrote {SOCKET_UNIT_FILE} and {SERVICE_UNIT_FILE}"),
        format!("the account service listens at {HELPER_SOCKET}"),
    ])
}

fn remove(delete_remote_account: bool) -> io::Result<Vec<String>> {
    let manager = manager()?;
    socket_restart::stop_inactive(&manager)?;
    exec_systemd::disable_unit_file(&manager, SOCKET_UNIT)?;
    // Each instance stops the account units of its connection on SIGTERM.
    for instance in exec_systemd::unit_names(&manager, SERVICE_INSTANCES)? {
        exec_systemd::stop_unit(&manager, &instance)?;
    }
    let mut done = vec!["stopped the account service".to_string()];
    for file in [SOCKET_UNIT_FILE, SERVICE_UNIT_FILE] {
        match std::fs::remove_file(file) {
            Ok(()) => done.push(format!("removed {file}")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    exec_systemd::reload_manager(&manager)?;
    if exec_helper_config::remove()? {
        done.push(format!("removed {CONFIG_FILE}"));
    }
    if exec_pam_service::remove()? {
        done.push(format!("removed {PAM_FILE}"));
    }
    if exec_binary_copy::remove_copy()? {
        done.push(format!("removed {PROGRAM_COPY}"));
    }
    if !delete_remote_account {
        done.push("kept the account se-remote (--delete-remote-account deletes it)".into());
        return Ok(done);
    }
    if exec_remote_account::delete_remote_account()? {
        done.push("stopped the units of se-remote and deleted it with its home".into());
    }
    Ok(done)
}

/// An account an administrator may allow: it exists and is not root.
fn allowable(account: &str) -> io::Result<()> {
    match exec_account::check_account(account) {
        Ok(_) | Err(AccountRefusal::Expired | AccountRefusal::PasswordChangeRequired) => Ok(()),
        Err(refusal) => Err(invalid(refusal.detail(account))),
    }
}

fn manager() -> io::Result<Connection> {
    Connection::system().map_err(eio)
}

fn is_root() -> bool {
    // SAFETY: `geteuid` only reads this process's id.
    unsafe { libc::geteuid() == 0 }
}

fn not_set_up() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        "the account service is not set up; run `se share exec accounts setup` first",
    )
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[path = "exec_helper_socket.rs"]
mod socket_restart;
#[cfg(test)]
#[path = "exec_helper_tests.rs"]
mod tests;
