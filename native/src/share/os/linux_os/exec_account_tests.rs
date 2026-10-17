//! Acceptance signals of PRM-M5 (strang-rechte.md R 348, without the
//! property check of U01). Tests marked `ignore` run only in their job of
//! the remote suite `rechte-desktop-tresore-task`: `linux-root` runs them as
//! root with `SMART_EXPLORER_PERM_ROOT_TASK=1`, `linux-2204` on Ubuntu 22.04
//! with `SMART_EXPLORER_PERM_2204_TASK=1`; both name the built `se` in
//! `SMART_EXPLORER_PERM_SE_BIN`, whose `--share-exec-supervisor-stdio` runs
//! in the account units (the test program has no supervisor mode).
use std::collections::BTreeMap;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use zbus::blocking::Connection;

use crate::share::exec_helper_protocol::{account_error_code, ACCOUNT_UNAVAILABLE};
use crate::share::exec_platform::{ContainedExec, StopReason};
use crate::share::exec_supervisor_protocol::{
    recv_event, send_command, RunAsAccount, SupervisorCommand, SupervisorEvent, SupervisorStart,
};
use crate::share::exec_terminal::{ExecTerminalRequest, ExecTerminalSize};
use crate::share::exec_types::{ExecCommand, ExecId, ExecStart};

use super::super::exec_systemd::{self, manager_connection, unit_properties};
use super::super::{
    exec_account, exec_binary_copy, exec_pam_service, exec_remote_account, exec_supervisor,
};
use super::{AccountUnit, AccountUnitStart, UNIT_PREFIX};

const REMOTE: &str = "se-remote";
const WAIT: Duration = Duration::from_secs(30);

/// What a job printed and how it ended.
#[derive(Debug, Default)]
pub(crate) struct JobOutcome {
    pub(crate) stdout: String,
    pub(crate) exit: Option<i32>,
    pub(crate) error: Option<String>,
}

pub(crate) fn request(command: ExecCommand, terminal: bool) -> ExecStart {
    ExecStart {
        exec_id: ExecId::generate().expect("exec id"),
        command,
        cwd: None,
        env: BTreeMap::new(),
        timeout_ms: Some(120_000),
        max_output_bytes: None,
        terminal: terminal.then(|| ExecTerminalRequest {
            term: "xterm".into(),
            size: ExecTerminalSize {
                cols: 100,
                rows: 30,
            },
        }),
    }
}

pub(crate) fn shell(command: &str) -> ExecCommand {
    ExecCommand::Shell {
        command: command.into(),
    }
}

/// Prepares and commits `request` as `account` the way the Exec host does.
pub(crate) fn start(request: &ExecStart, account: &RunAsAccount) -> io::Result<ContainedExec> {
    let mut process = ContainedExec::prepare_as(request, Some(account))?;
    process.commit(request.clone(), Some(account.clone()), None)?;
    Ok(process)
}

/// Reads events until the output holds `needle`, the job ends or `WAIT`.
pub(crate) fn read_until(process: &mut ContainedExec, needle: &str, job: &mut JobOutcome) {
    let deadline = Instant::now() + WAIT;
    while !job.stdout.contains(needle) && job.exit.is_none() && job.error.is_none() {
        match process.next_event(Some(deadline)) {
            Ok(SupervisorEvent::Stdout(bytes) | SupervisorEvent::Stderr(bytes)) => {
                job.stdout.push_str(&String::from_utf8_lossy(&bytes));
            }
            Ok(SupervisorEvent::Exited(exit)) => job.exit = Some(exit.code.unwrap_or(-1)),
            Ok(SupervisorEvent::Error(message)) => job.error = Some(message),
            Ok(_) => {}
            Err(error) => job.error = Some(error.to_string()),
        }
    }
}

/// Runs a whole job as `account` and waits until its unit is empty.
pub(crate) fn run_job(request: ExecStart, account: &RunAsAccount, stdin: &[u8]) -> JobOutcome {
    let mut process = start(&request, account).expect("start the job");
    if !stdin.is_empty() {
        process.write_stdin(stdin).expect("terminal input");
    }
    let mut job = JobOutcome::default();
    read_until(&mut process, "\u{0}never\u{0}", &mut job);
    process.terminate_all(StopReason::RootExited).expect("stop");
    process
        .confirm_empty(Instant::now() + WAIT)
        .expect("the account unit ends empty");
    job
}

/// Pids of every process that runs with `uid`.
pub(crate) fn processes_of(uid: u32) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            std::fs::read_to_string(format!("/proc/{pid}/status")).is_ok_and(|status| {
                status.lines().any(|line| {
                    line.strip_prefix("Uid:")
                        .and_then(|ids| ids.split_whitespace().next())
                        .is_some_and(|real| real == uid.to_string())
                })
            })
        })
        .collect()
}

fn root_task() {
    assert_eq!(
        std::env::var("SMART_EXPLORER_PERM_ROOT_TASK").as_deref(),
        Ok("1"),
        "runs only in the linux-root job of rechte-desktop-tresore-task"
    );
    // SAFETY: `geteuid` only reads this process's id.
    assert_eq!(
        unsafe { libc::geteuid() },
        0,
        "the linux-root job runs as root"
    );
    assert!(std::env::var_os("SMART_EXPLORER_PERM_SE_BIN").is_some());
}

/// `se-remote`, created by the code of "Fernzugriffskonto anlegen".
fn remote_account() -> RunAsAccount {
    exec_remote_account::ensure_remote_account().expect("create se-remote");
    exec_account::resolve_for_start(REMOTE).expect("se-remote passes the account checks")
}

/// A temporary account, changed by `adjust` (`usermod`/`chage` arguments).
struct TemporaryAccount(&'static str);

impl TemporaryAccount {
    fn create(name: &'static str, tool: &str, adjust: &[&str]) -> Self {
        let _ = Command::new("userdel").args(["--remove", name]).status();
        let created = Command::new("useradd")
            .args(["--create-home", "--shell", "/bin/sh", name])
            .status()
            .expect("useradd");
        assert!(created.success(), "useradd {name}");
        let adjusted = Command::new(tool).args(adjust).arg(name).status();
        assert!(adjusted.expect(tool).success(), "{tool} {adjust:?} {name}");
        Self(name)
    }
}

impl Drop for TemporaryAccount {
    fn drop(&mut self) {
        let _ = Command::new("userdel").args(["--remove", self.0]).status();
    }
}

/// A refused start of `name`: the admission's check names `reason`, and
/// PAM in the unit refuses it as well where the check is skipped.
fn assert_refused(name: &str, reason: &str) {
    exec_pam_service::ensure_installed().expect("PAM service file");
    let error = exec_account::resolve_for_start(name).expect_err("the account is refused");
    assert_eq!(account_error_code(&error), Some(ACCOUNT_UNAVAILABLE));
    assert!(error.to_string().contains(reason), "{error}");
    let program = exec_binary_copy::program_for_units().expect("program");
    let (_worker, supervisor) = UnixStream::pair().expect("socket pair");
    let connection = Connection::system().expect("system bus");
    let unit = format!(
        "{UNIT_PREFIX}{}.service",
        ExecId::generate().unwrap().as_str()
    );
    let started = AccountUnit::start(
        connection,
        unit,
        &AccountUnitStart {
            account: name,
            cwd: None,
            program: &program,
            runtime_usec: None,
            stdio: supervisor.as_fd(),
        },
    );
    let error = started.err().expect("PAM refuses the account unit");
    assert!(error.to_string().contains("PAM"), "{error}");
}

#[test]
#[ignore = "runs in the linux-2204 job of rechte-desktop-tresore-task"]
fn perm_task_exec_linux_ubuntu2204_starts() {
    assert_eq!(
        std::env::var("SMART_EXPLORER_PERM_2204_TASK").as_deref(),
        Ok("1")
    );
    let se = std::env::var_os("SMART_EXPLORER_PERM_SE_BIN").expect("SMART_EXPLORER_PERM_SE_BIN");
    let connection = manager_connection().expect("the worker's systemd manager");
    let version: String = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )
    .and_then(|manager| manager.get_property("Version"))
    .expect("systemd version");
    assert!(version.starts_with("249"), "systemd {version}, not 249");
    // The platform self-test of `se` runs jobs in the worker's own transient
    // units, which systemd 249 refused while they carried `ExitType`.
    let output = Command::new(se)
        .arg("--share-exec-platform-self-test")
        .output()
        .expect("se self-test");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "needs root: linux-root job of rechte-desktop-tresore-task"]
fn perm_task_remote_account_ls_runs_linux_root() {
    root_task();
    let account = remote_account();
    let job = run_job(
        request(shell("id -un; ls / >/dev/null && echo LS-OK"), false),
        &account,
        b"",
    );
    assert_eq!(job.error, None);
    assert_eq!(job.exit, Some(0), "{job:?}");
    let lines: Vec<&str> = job.stdout.lines().collect();
    assert!(lines.ends_with(&[REMOTE, "LS-OK"]), "{job:?}");
}

#[test]
#[ignore = "needs root: linux-root job of rechte-desktop-tresore-task"]
fn perm_task_linux_root_pam_session() {
    root_task();
    let account = remote_account();
    let limits = Path::new("/etc/security/limits.d/perm-task.conf");
    std::fs::write(limits, format!("{REMOTE} - nofile 4321\n")).expect("limits file");
    let job = run_job(
        request(shell("cat /proc/self/loginuid; echo; ulimit -n"), false),
        &account,
        b"",
    );
    let _ = std::fs::remove_file(limits);
    let lines: Vec<&str> = job.stdout.lines().collect();
    let uid = account.uid.to_string();
    assert!(lines.ends_with(&[uid.as_str(), "4321"]), "{job:?}");
}

#[test]
#[ignore = "needs root: linux-root job of rechte-desktop-tresore-task"]
fn perm_task_linux_root_expired_account_refused() {
    root_task();
    let _account = TemporaryAccount::create("perm-task-expired", "usermod", &["--expiredate", "1"]);
    assert_refused("perm-task-expired", "expired or locked");
}

#[test]
#[ignore = "needs root: linux-root job of rechte-desktop-tresore-task"]
fn perm_task_linux_password_change_required_refused() {
    root_task();
    let _account = TemporaryAccount::create("perm-task-change", "chage", &["--lastday", "0"]);
    assert_refused("perm-task-change", "requires a password change");
}

#[test]
#[ignore = "needs root: linux-root job of rechte-desktop-tresore-task"]
fn perm_task_linux_terminal_as_account() {
    root_task();
    let account = remote_account();
    let job = run_job(
        request(ExecCommand::Login, true),
        &account,
        b"echo TERMINAL-$(id -un)-$(tty | cut -c1-9)\nexit 0\n",
    );
    assert_eq!(job.exit, Some(0), "{job:?}");
    assert!(
        job.stdout.contains("TERMINAL-se-remote-/dev/pts/"),
        "{job:?}"
    );
}

#[test]
#[ignore = "needs root: linux-root job of rechte-desktop-tresore-task"]
fn perm_task_linux_account_unit_contains_children() {
    root_task();
    let account = remote_account();
    let request = request(shell("setsid sleep 300 & echo CHILD=$!; sleep 300"), false);
    let mut process = start(&request, &account).expect("start");
    let mut job = JobOutcome::default();
    read_until(&mut process, "\n", &mut job);
    let child = job
        .stdout
        .trim()
        .strip_prefix("CHILD=")
        .and_then(|pid| pid.parse::<u32>().ok())
        .unwrap_or_else(|| panic!("no child pid: {job:?}"));
    let unit = format!("/{UNIT_PREFIX}{}.service", request.exec_id.as_str());
    let cgroup = std::fs::read_to_string(format!("/proc/{child}/cgroup")).expect("cgroup");
    assert!(cgroup.trim_end().ends_with(&unit), "{cgroup} lacks {unit}");
    process
        .terminate_all(StopReason::Cancelled)
        .expect("cancel");
    process
        .confirm_empty(Instant::now() + WAIT)
        .expect("the unit ends empty");
    assert!(!Path::new(&format!("/proc/{child}")).exists());
    assert_eq!(processes_of(account.uid), Vec::<u32>::new());
}

#[test]
#[ignore = "needs root: linux-root job of rechte-desktop-tresore-task"]
fn perm_task_linux_cancel_contains_account_unit() {
    root_task();
    let account = remote_account();
    let request = request(
        shell(
            "trap '' TERM HUP; setsid sh -c \"trap '' TERM HUP; sleep 300\" & \
             echo READY; sleep 300",
        ),
        false,
    );
    let mut process = start(&request, &account).expect("start");
    let mut job = JobOutcome::default();
    read_until(&mut process, "READY", &mut job);
    assert!(job.stdout.contains("READY"), "{job:?}");
    let begun = Instant::now();
    process
        .terminate_all(StopReason::Cancelled)
        .expect("cancel");
    process
        .confirm_empty(begun + Duration::from_secs(10))
        .expect("cancel empties the account unit within 10 s");
    let name = format!("{UNIT_PREFIX}{}.service", request.exec_id.as_str());
    let connection = Connection::system().expect("system bus");
    if let Some(path) = exec_systemd::loaded_unit(&connection, &name).expect("GetUnit") {
        let state = exec_systemd::unit_active_state(&connection, &path).unwrap_or_default();
        assert!(
            matches!(state.as_str(), "inactive" | "failed" | ""),
            "{state}"
        );
    }
    assert_eq!(processes_of(account.uid), Vec::<u32>::new());
}

#[test]
#[ignore = "needs root: linux-root job of rechte-desktop-tresore-task"]
fn perm_task_remote_account_cannot_read_ipc_token_linux() {
    root_task();
    let account = remote_account();
    // The worker keeps `daemon.token` like this file: in its private data
    // directory, written by the same function.
    let directory = crate::support_dirs::app_data_dir();
    let probe = directory.join("perm-task-token-probe");
    crate::support_dirs::write_private_atomic(&probe, b"PERM-TASK-SECRET").expect("probe");
    let mut script = format!("cat '{}'; echo STATUS=$?", probe.display());
    let token = directory.join("daemon.token");
    if token.exists() {
        script.push_str(&format!(
            "; cat '{}' >/dev/null; echo TOKEN=$?",
            token.display()
        ));
    }
    let job = run_job(request(shell(&script), false), &account, b"");
    let _ = std::fs::remove_file(&probe);
    assert!(!job.stdout.contains("PERM-TASK-SECRET"), "{job:?}");
    assert!(
        job.stdout.contains("STATUS=1"),
        "the remote account read the worker's file: {job:?}"
    );
    assert!(!job.stdout.contains("TOKEN=0"), "{job:?}");
}

/// Preserved behaviour (D3, S1): the worker's own account keeps its unit
/// of every release before PRM-M5, `prepare` takes that path outside
/// `prepare_as`, and the supervisor runs the job as before but never
/// switches accounts itself.
#[test]
fn perm_task_own_account_path_unchanged() {
    let socket = "/run/user/1000/smart-explorer-exec/0c.sock";
    for runtime in [None, Some(5_000_000)] {
        let properties = unit_properties(
            "/proc/7/exe".into(),
            socket.into(),
            runtime,
            "inactive-or-failed",
        )
        .expect("own unit");
        let names: Vec<&str> = properties.iter().map(|(name, _)| *name).collect();
        let mut expected = vec![
            "Description",
            "Type",
            "ExecStart",
            "KillMode",
            "KillSignal",
            "SendSIGKILL",
            "Restart",
            "OOMPolicy",
            "TimeoutStartUSec",
            "TimeoutStopUSec",
            "CollectMode",
        ];
        if runtime.is_some() {
            expected.push("RuntimeMaxUSec");
        }
        assert_eq!(names, expected);
        let exec_start = format!("{:?}", properties[2].1);
        assert!(
            exec_start.contains("--share-exec-supervisor"),
            "{exec_start}"
        );
        assert!(!exec_start.contains("-stdio"), "{exec_start}");
        assert!(exec_start.contains(socket), "{exec_start}");
    }
    assert!(crate::share::exec_platform::preparing_account().is_none());

    // SAFETY: the two calls only read this process's ids.
    let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
    let own = supervise(None);
    assert!(own.contains(&format!("UID={uid}")), "{own}");
    let other = RunAsAccount {
        name: "perm-task-other".into(),
        uid: uid.wrapping_add(4711),
        gid,
        groups: vec![gid],
        home: "/".into(),
        shell: "/bin/sh".into(),
    };
    let refused = supervise(Some(other));
    assert!(
        refused.contains("does not run as the job's account"),
        "{refused}"
    );
}

/// Runs `id -u` under an in-process supervisor; returns its output or error.
fn supervise(account: Option<RunAsAccount>) -> String {
    let (mut worker, supervisor) = UnixStream::pair().expect("socket pair");
    let thread =
        std::thread::spawn(move || exec_supervisor::run(supervisor, &super::super::LINUX_SPAWN));
    let request = request(shell("echo UID=$(id -u)"), false);
    let environment = BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]);
    send_command(
        &mut worker,
        &SupervisorCommand::Start(SupervisorStart {
            request,
            environment,
            account,
        }),
    )
    .expect("start frame");
    worker.set_read_timeout(Some(WAIT)).expect("timeout");
    let mut text = String::new();
    loop {
        match recv_event(&mut worker) {
            Ok(SupervisorEvent::Stdout(bytes)) => text.push_str(&String::from_utf8_lossy(&bytes)),
            Ok(SupervisorEvent::Error(message)) => {
                text.push_str(&message);
                break;
            }
            Ok(SupervisorEvent::Exited(_)) | Err(_) => break,
            Ok(_) => {}
        }
    }
    drop(worker);
    let _ = thread.join();
    text
}
