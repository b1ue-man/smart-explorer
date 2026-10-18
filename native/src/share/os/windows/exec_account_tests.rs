//! Acceptance signals of PRM-M7 (strang-rechte.md R 370). They change the
//! system (service, accounts, configuration) and run only in the stage
//! `perm-windows` of the job `windows-core` of the remote suite
//! `rechte-desktop-tresore-task`: on an elevated Windows runner (A12), with
//! `SMART_EXPLORER_PERM_WINDOWS_TASK=1`, the built debug `se.exe` in
//! `SMART_EXPLORER_PERM_SE_BIN`, `--include-ignored` and one test thread.
//! The runner is elevated, so the hidden setup mode runs without a UAC
//! prompt; jobs run through the production path of the worker
//! (`ContainedExec::prepare_as`, the account service, the relay). The
//! signals about running jobs are in `exec_account_job_tests.rs`.
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::File;
use std::io::Write;
use std::os::windows::io::FromRawHandle;
use std::path::PathBuf;
use std::process::Command;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::{
    ImpersonateLoggedOnUser, LogonUserW, RevertToSelf, LOGON32_LOGON_NETWORK,
    LOGON32_PROVIDER_DEFAULT,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, OPEN_EXISTING, SECURITY_ANONYMOUS, SECURITY_SQOS_PRESENT,
};

use crate::share::access_catalog::HelperState;
use crate::share::access_grant::REMOTE_ACCOUNT_NAME;
use crate::share::exec_client_messages::ACCOUNT_NOT_ALLOWED;
use crate::share::exec_helper_protocol::{
    account_error_code, read_frame, AccountChoiceProfile, AccountReason, HelperErrorCode,
    HelperReply,
};
use crate::share::exec_platform::{preparing_account, ContainedExec, StopReason};
use crate::share::exec_supervisor_protocol::{RunAsAccount, SupervisorEvent};
use crate::share::exec_terminal::{ExecTerminalRequest, ExecTerminalSize};
use crate::share::exec_types::{ExecCommand, ExecId, ExecStart};

use super::exec_account_choices::account_choices;
use super::exec_account_client::{
    own_sid, resolve_account, AccountSession, ProbeOutcome, ServiceConnection,
};
use super::exec_account_config::{config_path, load_config, program_copy, SERVICE_PIPE};
use super::exec_account_files::{sha256_file, write_trusted, CONFIG_SDDL};
use super::exec_account_pipes::CLIENT_ACCESS;
use super::exec_account_scm::service_state;
use super::exec_account_security::{computer_name, process_is_elevated, wide};
use super::exec_account_setup::{mode_arguments, ExecAccountsAction};
use super::exec_handles::OwnedHandle;

pub(super) const WAIT: Duration = Duration::from_secs(60);
const UNLISTED: &str = "perm-task-unlisted";
const CALLER: &str = "perm-task-caller";
const ADMIN: &str = "perm-task-admin";

pub(super) struct WindowsTask {
    se: PathBuf,
    sid: String,
}

pub(super) fn task() -> WindowsTask {
    assert_eq!(
        std::env::var("SMART_EXPLORER_PERM_WINDOWS_TASK").as_deref(),
        Ok("1"),
        "runs only in the windows-core job (perm-windows) of rechte-desktop-tresore-task"
    );
    assert!(process_is_elevated(), "the runner must be elevated (A12)");
    let se = std::env::var_os("SMART_EXPLORER_PERM_SE_BIN").expect("SMART_EXPLORER_PERM_SE_BIN");
    WindowsTask {
        se: PathBuf::from(se),
        sid: own_sid().expect("own SID"),
    }
}

/// Runs `action` in the hidden setup mode of the built `se.exe`.
fn administer(task: &WindowsTask, action: &ExecAccountsAction) -> (bool, String) {
    let hash = sha256_file(&task.se).expect("hash of se.exe");
    let output = Command::new(&task.se)
        .args(mode_arguments(action, &task.sid, &hash))
        .output()
        .expect("se.exe");
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    (output.status.success(), text)
}

pub(super) fn set_up(task: &WindowsTask) -> String {
    let setup = ExecAccountsAction::Setup {
        remote_account: true,
        allow: Vec::new(),
    };
    let (done, report) = administer(task, &setup);
    assert!(done, "setup failed: {report}");
    report
}

fn run(program: &str, arguments: &[&str]) -> bool {
    Command::new(program)
        .args(arguments)
        .status()
        .is_ok_and(|status| status.success())
}

/// A local account `name` with a password that only this test knows.
fn local_account(name: &str) -> String {
    // At most 14 characters, so `net user` asks no question.
    let password = format!("Pt{}!x9", &ExecId::generate().unwrap().as_str()[..9]);
    let _ = run("net", &["user", name, "/delete"]);
    assert!(
        run("net", &["user", name, &password, "/add"]),
        "net user {name}"
    );
    password
}

pub(super) fn request(command: ExecCommand, terminal: bool) -> ExecStart {
    let terminal = terminal.then(|| ExecTerminalRequest {
        term: "xterm-256color".into(),
        size: ExecTerminalSize {
            cols: 120,
            rows: 30,
        },
    });
    ExecStart {
        exec_id: ExecId::generate().unwrap(),
        command,
        cwd: None,
        env: BTreeMap::new(),
        timeout_ms: Some(120_000),
        max_output_bytes: None,
        terminal,
    }
}

pub(super) fn shell(command: &str) -> ExecCommand {
    ExecCommand::Shell {
        command: command.into(),
    }
}

pub(super) struct JobResult {
    pub(super) stdout: String,
    pub(super) exit: Option<i32>,
    pub(super) error: Option<String>,
}

/// Runs `request` as `account` through the worker's production path;
/// `input` goes to the job's standard input (or terminal) after its start.
pub(super) fn run_job(request: ExecStart, account: &RunAsAccount, input: &[u8]) -> JobResult {
    let mut process =
        ContainedExec::prepare_as(&request, Some(account)).expect("prepared by the service");
    process
        .commit(request, Some(account.clone()), None)
        .expect("committed");
    if !input.is_empty() {
        assert_eq!(process.write_stdin(input).expect("input"), input.len());
    }
    let deadline = Instant::now() + WAIT;
    let (mut stdout, mut exit, mut error) = (Vec::new(), None, None);
    loop {
        match process.next_event(Some(deadline)) {
            Ok(SupervisorEvent::Stdout(bytes) | SupervisorEvent::Stderr(bytes)) => {
                stdout.extend(bytes)
            }
            Ok(SupervisorEvent::Exited(status)) => {
                exit = status.code;
                break;
            }
            Ok(SupervisorEvent::Error(message)) => {
                error = Some(message);
                break;
            }
            Ok(_) => {}
            Err(failure) => {
                error = Some(failure.to_string());
                break;
            }
        }
    }
    process.terminate_all(StopReason::RootExited).expect("stop");
    process
        .confirm_empty(Instant::now() + WAIT)
        .expect("the session's Job became empty");
    JobResult {
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        exit,
        error,
    }
}

/// Image names of the processes that run as `account`, as `tasklist` of an
/// administrator sees them.
pub(super) fn processes_of(account: &str) -> Vec<String> {
    let filter = format!("USERNAME eq {}\\{account}", computer_name().unwrap());
    let output = Command::new("tasklist")
        .args(["/FI", &filter, "/FO", "CSV", "/NH"])
        .output()
        .expect("tasklist");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.starts_with('"'))
        .filter_map(|line| line.split('"').nth(1).map(str::to_string))
        .collect()
}

pub(super) fn wait_until(mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    done()
}

pub(super) fn remote_account() -> RunAsAccount {
    resolve_account(REMOTE_ACCOUNT_NAME).expect("se-remote passes the admission")
}

#[test]
#[ignore = "needs an elevated runner and the account service: windows-core job (perm-windows)"]
fn perm_task_windows_helper_setup_remote_account() {
    let task = task();
    let report = set_up(&task);
    assert!(service_state().unwrap().is_some(), "{report}");
    let copy = program_copy().unwrap();
    assert_eq!(sha256_file(&copy).unwrap(), sha256_file(&task.se).unwrap());
    let config = load_config().unwrap().expect("configuration");
    assert!(config.allows(&task.sid, REMOTE_ACCOUNT_NAME), "{config:?}");
    let choices = account_choices();
    assert_eq!(choices.helper.state, HelperState::Installed, "{choices:?}");
    let remote = choices
        .choices
        .iter()
        .find(|choice| choice.profile == AccountChoiceProfile::Remote)
        .expect("the choice se-remote");
    assert!(remote.available, "{remote:?}");
    let job = run_job(
        request(shell("whoami & dir C:\\ >nul && echo LS-OK"), false),
        &remote_account(),
        b"",
    );
    assert_eq!(job.exit, Some(0), "{:?} {}", job.error, job.stdout);
    let expected = format!("{}\\se-remote", computer_name().unwrap()).to_lowercase();
    assert!(
        job.stdout.to_lowercase().contains(&expected),
        "{}",
        job.stdout
    );
    assert!(job.stdout.contains("LS-OK"), "{}", job.stdout);
}

#[test]
#[ignore = "needs an elevated runner and the account service: windows-core job (perm-windows)"]
fn perm_task_windows_helper_refuses_unlisted_account() {
    let task = task();
    set_up(&task);
    local_account(UNLISTED);
    let error = resolve_account(UNLISTED).expect_err("the admission refuses");
    assert_eq!(
        account_error_code(&error),
        Some(ACCOUNT_NOT_ALLOWED),
        "{error}"
    );
    let mut connection = ServiceConnection::open().ok().expect("connection");
    match connection.probe(UNLISTED).expect("probe") {
        ProbeOutcome::Refused(HelperErrorCode::NotAllowed, _) => {}
        ProbeOutcome::Refused(code, detail) => panic!("{code:?}: {detail}"),
        ProbeOutcome::Measured { .. } => panic!("the service probed an unlisted account"),
    }
    let mut unlisted = remote_account();
    unlisted.name = UNLISTED.into();
    let start = request(shell("whoami"), false);
    let error = AccountSession::start(&start, &unlisted)
        .err()
        .expect("the service refuses the start");
    assert_eq!(
        account_error_code(&error),
        Some(ACCOUNT_NOT_ALLOWED),
        "{error}"
    );
    // An administrator account is never allowed (C2, hard rule).
    let allow = ExecAccountsAction::Allow {
        account: "Administrator".into(),
    };
    let (done, report) = administer(&task, &allow);
    assert!(!done, "{report}");
    let _ = run("net", &["user", UNLISTED, "/delete"]);
}

#[test]
#[ignore = "needs an elevated runner and the account service: windows-core job (perm-windows)"]
fn perm_task_windows_pipe_caller_impersonation() {
    let task = task();
    set_up(&task);
    drop(ServiceConnection::open().ok().expect("the service runs"));
    // A user the configuration does not list cannot even open the pipe,
    // whatever process id it might name.
    let password = local_account(CALLER);
    let (name, domain, secret) = (wide(CALLER), wide("."), wide(&password));
    let mut token = null_mut();
    let logged_on = unsafe {
        LogonUserW(
            name.as_ptr(),
            domain.as_ptr(),
            secret.as_ptr(),
            LOGON32_LOGON_NETWORK,
            LOGON32_PROVIDER_DEFAULT,
            &mut token,
        )
    };
    assert_ne!(logged_on, 0, "{}", std::io::Error::last_os_error());
    let token = OwnedHandle::new(token).unwrap();
    assert_ne!(unsafe { ImpersonateLoggedOnUser(token.raw()) }, 0);
    let opened = open_service_pipe(SECURITY_SQOS_PRESENT | SECURITY_ANONYMOUS);
    assert_ne!(unsafe { RevertToSelf() }, 0);
    let error = opened.err().expect("the pipe refuses an unlisted user");
    assert_eq!(error.raw_os_error(), Some(ERROR_ACCESS_DENIED as i32));
    // A registered process that hides its identity (anonymous) and names a
    // registered process id is refused: only the kernel's caller counts.
    let mut pipe = open_service_pipe(SECURITY_SQOS_PRESENT | SECURITY_ANONYMOUS)
        .expect("a registered user opens the pipe");
    let hello = format!(
        r#"{{"hello":{{"protocol":1,"se_version":"perm-task","pid":{}}}}}"#,
        std::process::id()
    );
    let mut frame = (hello.len() as u32).to_be_bytes().to_vec();
    frame.extend_from_slice(hello.as_bytes());
    pipe.write_all(&frame).unwrap();
    let reply: HelperReply = read_frame(&mut pipe).unwrap().expect("a reply");
    assert!(
        matches!(
            reply,
            HelperReply::Error {
                code: HelperErrorCode::NotAllowed,
                ..
            }
        ),
        "{reply:?}"
    );
    let _ = run("net", &["user", CALLER, "/delete"]);
}

fn open_service_pipe(flags: u32) -> std::io::Result<File> {
    let name = wide(OsStr::new(SERVICE_PIPE));
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            CLIENT_ACCESS,
            0,
            null(),
            OPEN_EXISTING,
            flags,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_handle(handle.cast()) })
}

#[test]
#[ignore = "needs an elevated runner and the account service: windows-core job (perm-windows)"]
fn perm_task_windows_probe_reports_token_level() {
    let task = task();
    set_up(&task);
    local_account(ADMIN);
    let administrators = super::exec_account_install::account_name_of(
        &super::exec_account_security::Sid::well_known(
            windows_sys::Win32::Security::WinBuiltinAdministratorsSid,
        )
        .unwrap(),
    )
    .unwrap();
    assert!(run("net", &["localgroup", &administrators, ADMIN, "/add"]));
    // Setup never allows an administrator; the test lists it directly to
    // measure what the probe reports.
    let mut config = load_config().unwrap().expect("configuration");
    config.allow(&task.sid, ADMIN).unwrap();
    write_trusted(&config_path().unwrap(), &config.render(), CONFIG_SDDL).unwrap();
    let mut connection = ServiceConnection::open().ok().expect("connection");
    let elevated = match connection.probe(ADMIN).expect("probe") {
        ProbeOutcome::Measured {
            ok,
            elevated,
            admin_group,
            ..
        } => {
            assert!(!ok, "an administrator is never a target");
            assert!(admin_group);
            elevated
        }
        ProbeOutcome::Refused(code, detail) => panic!("{code:?}: {detail}"),
    };
    // A9: a local administrator logs on filtered over the network, where
    // Windows filters such logons at all.
    if remote_filtering() {
        assert!(
            !elevated,
            "A9: the network logon of an administrator is filtered"
        );
    } else {
        eprintln!(
            "A9 not checked: this runner does not filter administrator logons over the \
             network (EnableLUA 0 or LocalAccountTokenFilterPolicy 1); elevated={elevated}"
        );
    }
    let choices = account_choices();
    let admin = choices
        .choices
        .iter()
        .find(|choice| choice.label == ADMIN)
        .expect("the listed administrator");
    assert!(!admin.available, "{admin:?}");
    assert_eq!(admin.reason_code, Some(AccountReason::Root));
    assert_eq!(
        (admin.elevated, admin.admin_group),
        (Some(elevated), Some(true))
    );
    config.disallow(&task.sid, ADMIN);
    write_trusted(&config_path().unwrap(), &config.render(), CONFIG_SDDL).unwrap();
    let _ = run("net", &["user", ADMIN, "/delete"]);
}

/// Whether Windows filters the network logons of local administrators (UAC
/// on, no `LocalAccountTokenFilterPolicy`): the premise of A9.
fn remote_filtering() -> bool {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    let key = winreg::RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System");
    let value = |name: &str| {
        key.as_ref()
            .ok()
            .and_then(|key| key.get_value::<u32, _>(name).ok())
    };
    value("EnableLUA").unwrap_or(1) != 0 && value("LocalAccountTokenFilterPolicy").unwrap_or(0) != 1
}

#[test]
#[ignore = "needs the built se.exe: windows-core job (perm-windows)"]
fn perm_task_windows_own_account_path_unchanged() {
    let task = task();
    // Outside `prepare_as` no start is prepared for another account.
    assert!(preparing_account().is_none());
    // The own account's path: Job Object, supervisor, ConPTY and handle
    // allow-list self-tests of the debug program, exactly as before PRM-M7.
    let output = Command::new(&task.se)
        .arg("--share-exec-platform-self-test")
        .output()
        .expect("se.exe");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let own = &account_choices().choices[0];
    assert_eq!(own.profile, AccountChoiceProfile::Own);
    assert!(own.available);
    assert_eq!(
        own.label,
        std::env::var("USERNAME").unwrap_or_else(|_| "Windows user".into())
    );
}
