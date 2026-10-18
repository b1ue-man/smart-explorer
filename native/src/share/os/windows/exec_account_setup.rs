//! Setting up the Windows account service (PRM-M7, D.5): what `se share
//! exec accounts setup|allow|disallow|update|remove` asks for. The work
//! itself needs administrator rights (`exec_account_install.rs`). A process
//! without them asks with one UAC prompt: it starts `se.exe` beside itself
//! (else itself) elevated in the hidden mode `--exec-accounts-setup` or
//! `--exec-accounts-remove`, naming the user it acts for (`--for-sid`) and
//! the SHA-256 of that program (`--expect-sha256`). The elevated process
//! compares the hash with its own file before it does anything, and the
//! caller keeps the file open without write sharing meanwhile. The elevated
//! process leaves its report where only that user and administrators read
//! it. Where nobody confirms the prompt, or no prompt may be shown, the
//! caller gets the command a person runs in a terminal opened as
//! administrator. The types and the words are those of the Linux setup
//! (`linux_os/exec_helper_setup.rs`), so the CLI and the desktop use one API.
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io;
use std::mem::size_of;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use windows_sys::Win32::Foundation::ERROR_CANCELLED;
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows_sys::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS,
    SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

use super::exec_account_config::report_path;
use super::exec_account_files::{
    ensure_data_dir, read_trusted, report_sddl, sha256_file, sha256_reader, write_trusted,
};
use super::exec_account_install::apply;
use super::exec_account_security::{current_user_sid, process_is_elevated, sid_text, wide};
use super::exec_handles::OwnedHandle;

const SETUP_MODE: &str = "--exec-accounts-setup";
const REMOVE_MODE: &str = "--exec-accounts-remove";
const MAX_REPORT_BYTES: u64 = 64 * 1024;

/// What a person asks for (D.2.4, `se share exec accounts …`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExecAccountsAction {
    /// Installs or renews the account service and allows the user it is
    /// for `se-remote` (with `remote_account`, which also creates it) and
    /// `allow`.
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
    /// Renews the program copy and the service after an update.
    Update,
    /// Removes the account service; `se-remote` only on explicit request.
    Remove {
        delete_remote_account: bool,
    },
}

/// How a process without administrator rights asks for them. The names
/// follow the Linux API; on Windows both `Pkexec…` variants show the UAC
/// prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Elevation {
    PkexecGraphical,
    PkexecTerminal,
    /// Never show a prompt; return the command for an administrator.
    SudoCommandOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExecAccountsOutcome {
    /// Done: one line per step.
    Done(Vec<String>),
    /// Administrator rights are missing and could not be asked for here:
    /// the command a person runs, and why.
    NeedsAdministrator { command: String, reason: String },
}

/// Runs `action` as administrator: directly in an elevated process (for
/// the user of this process), otherwise after a UAC prompt as `elevation`
/// allows.
pub(crate) fn exec_accounts_apply(
    action: &ExecAccountsAction,
    elevation: Elevation,
) -> io::Result<ExecAccountsOutcome> {
    let sid = current_user_sid()?.text()?;
    let program = elevation_program()?;
    if process_is_elevated() {
        return apply(action, &sid, &program, None).map(ExecAccountsOutcome::Done);
    }
    let needs = |reason: &str| ExecAccountsOutcome::NeedsAdministrator {
        command: admin_command(&program, action),
        reason: reason.into(),
    };
    if elevation == Elevation::SudoCommandOnly {
        return Ok(needs(
            "administrator rights are needed: run the command in a terminal opened with \
             \"Run as administrator\" while signed in as this Windows user",
        ));
    }
    match run_elevated(&program, action, &sid)? {
        Some(lines) => Ok(ExecAccountsOutcome::Done(lines)),
        None => Ok(needs(
            "nobody confirmed the administrator request (UAC); run the command in a terminal \
             opened with \"Run as administrator\" while signed in as this Windows user",
        )),
    }
}

/// `se.exe` beside the running program where it exists (the desktop app
/// has it next to itself), else the running program.
fn elevation_program() -> io::Result<PathBuf> {
    let running = std::env::current_exe()?;
    let beside = running.with_file_name("se.exe");
    Ok(if beside.is_file() { beside } else { running })
}

/// Starts `program` elevated in the hidden mode and waits for it; `None`
/// when nobody confirmed the prompt.
fn run_elevated(
    program: &Path,
    action: &ExecAccountsAction,
    sid: &str,
) -> io::Result<Option<Vec<String>>> {
    // No one may change the program between the hash and the elevated run.
    let mut source = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(program)?;
    let hash = sha256_reader(&mut source)?;
    let parameters = mode_arguments(action, sid, &hash).join(" ");
    let (verb, file, parameters) = (wide("runas"), wide(program), wide(&parameters));
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = parameters.as_ptr();
    info.nShow = SW_HIDE;
    let started = SystemTime::now();
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_CANCELLED as i32) {
            return Ok(None);
        }
        return Err(error);
    }
    let process = OwnedHandle::new(info.hProcess)?;
    unsafe { WaitForSingleObject(process.raw(), INFINITE) };
    let mut code = 1u32;
    unsafe { GetExitCodeProcess(process.raw(), &mut code) };
    drop(source);
    let report = read_report(sid, started);
    if code == 0 {
        return Ok(Some(report.unwrap_or_default()));
    }
    let text = report
        .filter(|lines| !lines.is_empty())
        .map(|lines| lines.join("; "))
        .unwrap_or_else(|| format!("the elevated setup ended with exit code {code}"));
    Err(io::Error::other(text))
}

/// The lines of the report the elevated run wrote for `sid` after `since`.
fn read_report(sid: &str, since: SystemTime) -> Option<Vec<String>> {
    let path = report_path(sid).ok()?;
    let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
    if modified + Duration::from_secs(2) < since {
        return None;
    }
    let bytes = read_trusted(&path, MAX_REPORT_BYTES).ok()??;
    let text = String::from_utf8_lossy(&bytes);
    Some(text.lines().map(str::to_string).collect())
}

/// The hidden modes the UAC prompt runs: `--exec-accounts-setup --for-sid
/// SID --expect-sha256 HASH` with the words of `se share exec accounts`
/// (D.2.4), and `--exec-accounts-remove --for-sid SID --expect-sha256 HASH
/// [--delete-remote-account]`. Each step's line goes to standard output
/// and to the report.
pub(super) fn run_if_requested(arguments: &[OsString]) -> Option<io::Result<()>> {
    let first = arguments.first()?;
    if first != SETUP_MODE && first != REMOVE_MODE {
        return None;
    }
    Some(run_mode(arguments))
}

fn run_mode(arguments: &[OsString]) -> io::Result<()> {
    let (action, sid, expected) = parse_mode(arguments)?;
    let program = std::env::current_exe()?;
    let result = if sha256_file(&program)? == expected {
        apply(&action, &sid, &program, Some(&expected))
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the program changed after it was confirmed; nothing was changed",
        ))
    };
    let text = match &result {
        Ok(lines) => lines.join("\n"),
        Err(error) => format!("error: {error}"),
    };
    let reported = ensure_data_dir()
        .and_then(|_| write_trusted(&report_path(&sid)?, text.as_bytes(), &report_sddl(&sid)));
    if let Err(error) = reported {
        eprintln!("the report of the setup could not be written: {error}");
    }
    result.map(|lines| lines.iter().for_each(|line| println!("{line}")))
}

pub(super) fn parse_mode(
    arguments: &[OsString],
) -> io::Result<(ExecAccountsAction, String, String)> {
    let words = arguments
        .iter()
        .map(|word| {
            word.to_str()
                .ok_or_else(|| invalid("an argument is not UTF-8"))
        })
        .collect::<io::Result<Vec<&str>>>()?;
    let [mode, "--for-sid", sid, "--expect-sha256", hash, rest @ ..] = words.as_slice() else {
        return Err(invalid(
            "the setup mode needs --for-sid and --expect-sha256",
        ));
    };
    let hex = hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit());
    if !sid_text(sid) || !hex {
        return Err(invalid("invalid user or hash for the setup mode"));
    }
    let removing = *mode == REMOVE_MODE;
    let action = if removing {
        parse_words(&[&["remove"][..], rest].concat())?
    } else {
        parse_words(rest)?
    };
    if removing != matches!(action, ExecAccountsAction::Remove { .. }) {
        return Err(invalid(
            "the setup mode does not remove, the remove mode only removes",
        ));
    }
    Ok((action, sid.to_string(), hash.to_ascii_lowercase()))
}

/// `action` in the words of `se share exec accounts` (D.2.4).
pub(super) fn cli_words(action: &ExecAccountsAction) -> Vec<String> {
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

pub(super) fn parse_words(words: &[&str]) -> io::Result<ExecAccountsAction> {
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

/// The arguments of the hidden mode for `action`.
pub(super) fn mode_arguments(action: &ExecAccountsAction, sid: &str, hash: &str) -> Vec<String> {
    let mut words = cli_words(action);
    let mode = if matches!(action, ExecAccountsAction::Remove { .. }) {
        words.remove(0);
        REMOVE_MODE
    } else {
        SETUP_MODE
    };
    [mode, "--for-sid", sid, "--expect-sha256", hash]
        .into_iter()
        .map(str::to_string)
        .chain(words)
        .collect()
}

/// The command a person runs in PowerShell opened with "Run as
/// administrator", signed in as the user it is for (D.2.4 words).
pub(super) fn admin_command(program: &Path, action: &ExecAccountsAction) -> String {
    let program = program.display().to_string().replace('\'', "''");
    format!(
        "& '{program}' share exec accounts {}",
        cli_words(action).join(" ")
    )
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.to_string())
}
