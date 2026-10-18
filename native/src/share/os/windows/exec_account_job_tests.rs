//! Acceptance signals of PRM-M7 (strang-rechte.md R 370) about running
//! jobs as another account: the terminal, cancelling, the end of the
//! worker's connection and the worker's private files. Same stage and
//! switches as `exec_account_tests.rs`, whose helpers they use.
use std::collections::BTreeMap;
use std::time::Instant;

use crate::share::access_grant::REMOTE_ACCOUNT_NAME;
use crate::share::exec_platform::{ContainedExec, StopReason};
use crate::share::exec_supervisor_protocol::{send_command, SupervisorCommand, SupervisorStart};
use crate::share::exec_types::ExecCommand;

use super::exec_account_client::AccountSession;
use super::exec_account_tests::{
    processes_of, remote_account, request, run_job, set_up, shell, task, wait_until, WAIT,
};

#[test]
#[ignore = "needs an elevated runner and the account service: windows-core job (perm-windows)"]
fn perm_task_windows_terminal_as_remote_account() {
    let task = task();
    set_up(&task);
    let job = run_job(
        request(ExecCommand::Login, true),
        &remote_account(),
        b"(Get-Location).Path\r\nexit\r\n",
    );
    assert_eq!(job.error, None, "{}", job.stdout);
    // The relay starts in the profile folder of the (fresh) account (A8).
    let screen = job.stdout.to_lowercase();
    assert!(screen.contains("\\users\\se-remote"), "{}", job.stdout);
}

#[test]
#[ignore = "needs an elevated runner and the account service: windows-core job (perm-windows)"]
fn perm_task_windows_cancel_kills_job_tree() {
    let task = task();
    set_up(&task);
    let account = remote_account();
    let start = request(
        shell("start /b ping -n 600 127.0.0.1 >nul & ping -n 600 127.0.0.1 >nul"),
        false,
    );
    let mut process = ContainedExec::prepare_as(&start, Some(&account)).expect("prepared");
    process
        .commit(start, Some(account.clone()), None)
        .expect("committed");
    let pings = || {
        processes_of(REMOTE_ACCOUNT_NAME)
            .iter()
            .filter(|image| image.eq_ignore_ascii_case("ping.exe"))
            .count()
    };
    assert!(
        wait_until(|| pings() >= 2),
        "{:?}",
        processes_of(REMOTE_ACCOUNT_NAME)
    );
    process
        .terminate_all(StopReason::Cancelled)
        .expect("cancel");
    process
        .confirm_empty(Instant::now() + WAIT)
        .expect("ActiveProcesses reaches 0");
    assert!(
        wait_until(|| processes_of(REMOTE_ACCOUNT_NAME).is_empty()),
        "{:?}",
        processes_of(REMOTE_ACCOUNT_NAME)
    );
}

#[test]
#[ignore = "needs an elevated runner and the account service: windows-core job (perm-windows)"]
fn perm_task_windows_worker_disconnect_kills_jobs() {
    let task = task();
    set_up(&task);
    let account = remote_account();
    let start = request(shell("ping -n 600 127.0.0.1 >nul"), false);
    let (session, mut writer, _reader) =
        AccountSession::start(&start, &account).expect("a session as se-remote");
    let command = SupervisorCommand::Start(SupervisorStart {
        request: start,
        environment: BTreeMap::new(),
        account: Some(account),
    });
    send_command(&mut writer, &command).expect("start frame");
    assert!(wait_until(|| !processes_of(REMOTE_ACCOUNT_NAME).is_empty()));
    // The worker dies: its connection closes without a stop request.
    drop(session);
    assert!(
        wait_until(|| processes_of(REMOTE_ACCOUNT_NAME).is_empty()),
        "{:?}",
        processes_of(REMOTE_ACCOUNT_NAME)
    );
}

#[test]
#[ignore = "needs an elevated runner and the account service: windows-core job (perm-windows)"]
fn perm_task_remote_account_cannot_read_ipc_token_windows() {
    let task = task();
    set_up(&task);
    // Written like `daemon.token` (`write_private_atomic`, A4).
    let probe = crate::support_dirs::app_data_dir().join("perm-task-ipc-token-probe");
    crate::support_dirs::write_private_atomic(&probe, b"perm-task-secret").unwrap();
    let command = format!("type \"{}\"", probe.display());
    let job = run_job(request(shell(&command), false), &remote_account(), b"");
    let _ = std::fs::remove_file(&probe);
    assert_ne!(job.exit, Some(0), "{}", job.stdout);
    assert!(!job.stdout.contains("perm-task-secret"), "{}", job.stdout);
}
