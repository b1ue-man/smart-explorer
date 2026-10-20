//! Acceptance signals of PRM-M6 (strang-rechte.md R 359) and the hidden
//! setup modes. Tests marked `ignore` run only in the stage
//! `perm-linux-helper` of the job `linux-root` of the remote suite
//! `rechte-desktop-tresore-task`, with `SMART_EXPLORER_PERM_HELPER_TASK=1`,
//! as a user that is not root but may run `sudo -n`, and with the built
//! `se` in `SMART_EXPLORER_PERM_SE_BIN`. `sudo -n` runs the hidden mode
//! that `pkexec` runs on a desktop.
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use crate::share::access_catalog::HelperState;
use crate::share::exec_client_messages::ACCOUNT_NOT_ALLOWED;
use crate::share::exec_helper_protocol::{
    account_error_code, read_frame, write_frame, AccountChoiceProfile, AccountReason,
    HelperErrorCode, HelperReply, HelperRequest, HELPER_PROTOCOL,
};

use super::super::exec_account_unit::tests::{processes_of, request, run_job, shell};
use super::super::exec_helper_client::{self, HelperSession};
use super::super::exec_helper_server::HELPER_MODE;
use super::*;

const UNLISTED: &str = "perm-task-unlisted";
const WAIT: Duration = Duration::from_secs(20);
/// A client of another uid: `hello`, then the reply on standard output.
const FOREIGN_CLIENT: &str = r#"
import json, socket, struct, sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(sys.argv[1])
p = json.dumps({"hello": {"protocol": 1, "se_version": "perm-task"}}).encode()
s.sendall(struct.pack(">I", len(p)) + p)
def take(n):
    b = b""
    while len(b) < n:
        c = s.recv(n - len(b))
        if not c:
            raise SystemExit("closed")
        b += c
    return b
print(take(struct.unpack(">I", take(4))[0]).decode())
"#;

struct HelperTask {
    se: PathBuf,
    uid: u32,
}

fn helper_task() -> HelperTask {
    assert_eq!(
        std::env::var("SMART_EXPLORER_PERM_HELPER_TASK").as_deref(),
        Ok("1"),
        "runs only in the linux-root job (perm-linux-helper) of rechte-desktop-tresore-task"
    );
    // SAFETY: `geteuid` only reads this process's id.
    let uid = unsafe { libc::geteuid() };
    assert_ne!(
        uid, 0,
        "the account service serves workers that are not root"
    );
    let se = std::env::var_os("SMART_EXPLORER_PERM_SE_BIN").expect("SMART_EXPLORER_PERM_SE_BIN");
    HelperTask {
        se: PathBuf::from(se),
        uid,
    }
}

/// Runs `action` as administrator through the hidden mode `pkexec` runs.
fn administer(task: &HelperTask, action: &ExecAccountsAction) -> String {
    let output = Command::new("sudo")
        .arg("-n")
        .arg(&task.se)
        .args(mode_arguments(action, task.uid))
        .output()
        .expect("sudo");
    assert!(
        output.status.success(),
        "{action:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn set_up(task: &HelperTask) -> String {
    let setup = ExecAccountsAction::Setup {
        remote_account: true,
        allow: Vec::new(),
    };
    administer(task, &setup)
}

fn sudo(arguments: &[&str]) -> bool {
    Command::new("sudo")
        .arg("-n")
        .args(arguments)
        .status()
        .is_ok_and(|status| status.success())
}

fn unit_active(connection: &Connection, unit: &str) -> bool {
    let Ok(Some(path)) = exec_systemd::loaded_unit(connection, unit) else {
        return false;
    };
    let state = exec_systemd::unit_active_state(connection, &path).unwrap_or_default();
    !matches!(state.as_str(), "inactive" | "failed" | "")
}

fn wait_inactive(connection: &Connection, unit: &str) -> bool {
    let deadline = Instant::now() + WAIT;
    while unit_active(connection, unit) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    !unit_active(connection, unit)
}

fn session(supervisor: &UnixStream) -> HelperSession {
    HelperSession::start(REMOTE_ACCOUNT_NAME, None, supervisor).expect("a session as se-remote")
}

#[test]
#[ignore = "needs the account service: linux-root job (perm-linux-helper)"]
fn perm_task_linux_helper_setup_and_remote_account() {
    let task = helper_task();
    let report = set_up(&task);
    for path in [
        SOCKET_UNIT_FILE,
        SERVICE_UNIT_FILE,
        CONFIG_FILE,
        PAM_FILE,
        PROGRAM_COPY,
        HELPER_SOCKET,
    ] {
        assert!(Path::new(path).exists(), "{path} missing: {report}");
    }
    let choices = exec_remote_account::account_choices();
    assert_eq!(choices.helper.state, HelperState::Installed, "{choices:?}");
    let remote = choices
        .choices
        .iter()
        .find(|choice| choice.profile == AccountChoiceProfile::Remote)
        .expect("the choice se-remote");
    assert!(remote.available, "{remote:?}");
    let account = exec_account::resolve_for_start(REMOTE_ACCOUNT_NAME).expect("se-remote");
    let job = run_job(
        request(shell("id -un; ls / >/dev/null && echo LS-OK"), false),
        &account,
        b"",
    );
    assert_eq!(job.exit, Some(0), "{job:?}");
    let lines: Vec<&str> = job.stdout.lines().collect();
    assert!(lines.ends_with(&[REMOTE_ACCOUNT_NAME, "LS-OK"]), "{job:?}");
}

#[test]
#[ignore = "needs the account service: linux-root job (perm-linux-helper)"]
fn perm_task_linux_helper_refuses_unlisted_account() {
    let task = helper_task();
    set_up(&task);
    assert!(
        sudo(&["useradd", "--create-home", UNLISTED]) || exec_account::resolve(UNLISTED).is_ok()
    );
    let (_worker, supervisor) = UnixStream::pair().expect("socket pair");
    for account in [UNLISTED, "root"] {
        let error = HelperSession::start(account, None, &supervisor)
            .err()
            .expect("the account service refuses the account");
        assert_eq!(
            account_error_code(&error),
            Some(ACCOUNT_NOT_ALLOWED),
            "{error}"
        );
    }
    let error = exec_account::resolve_for_start(UNLISTED).expect_err("the admission refuses");
    assert_eq!(
        account_error_code(&error),
        Some(ACCOUNT_NOT_ALLOWED),
        "{error}"
    );
    sudo(&["userdel", "--remove", UNLISTED]);
}

#[test]
#[ignore = "needs the account service: linux-root job (perm-linux-helper)"]
fn perm_task_linux_helper_refuses_foreign_uid() {
    let task = helper_task();
    set_up(&task);
    let output = Command::new("sudo")
        .args([
            "-n",
            "-u",
            "nobody",
            "python3",
            "-c",
            FOREIGN_CLIENT,
            HELPER_SOCKET,
        ])
        .output()
        .expect("client as nobody");
    let text = String::from_utf8_lossy(&output.stdout);
    let reply: HelperReply = serde_json::from_str(text.trim()).expect("one reply");
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
}

#[test]
#[ignore = "needs the account service: linux-root job (perm-linux-helper)"]
fn perm_task_linux_helper_connection_close_stops_units() {
    let task = helper_task();
    set_up(&task);
    let account = exec_account::resolve_for_start(REMOTE_ACCOUNT_NAME).expect("se-remote");
    let (_worker, supervisor) = UnixStream::pair().expect("socket pair");
    let helper = session(&supervisor);
    drop(supervisor);
    let unit = helper.name().to_string();
    let connection = Connection::system().expect("system bus");
    assert!(unit_active(&connection, &unit), "{unit} does not run");
    drop(helper);
    assert!(
        wait_inactive(&connection, &unit),
        "{unit} outlived its connection"
    );
    assert_eq!(processes_of(account.uid), Vec::<u32>::new());
}

#[test]
#[ignore = "needs the account service: linux-root job (perm-linux-helper)"]
fn perm_task_linux_helper_hardened_unit_starts() {
    let task = helper_task();
    set_up(&task);
    let unit_file = std::fs::read_to_string(SERVICE_UNIT_FILE).expect("service unit");
    for line in [
        "ProtectSystem=strict",
        "ProtectHome=yes",
        "PrivateTmp=yes",
        "NoNewPrivileges=yes",
        "RestrictAddressFamilies=AF_UNIX",
        "CapabilityBoundingSet=",
    ] {
        assert!(unit_file.lines().any(|found| found == line), "{line}");
    }
    // A3: the instance without any capability starts the account unit.
    let (_worker, supervisor) = UnixStream::pair().expect("socket pair");
    let helper = session(&supervisor);
    let connection = Connection::system().expect("system bus");
    let instances = exec_systemd::unit_names(&connection, "smart-explorer-exec-helper@*.service")
        .expect("instances");
    let bounds: Vec<String> = instances
        .iter()
        .filter_map(|name| {
            let path = exec_systemd::loaded_unit(&connection, name).ok()??;
            let pid = exec_systemd::unit_main_pid(&connection, &path).ok()?;
            let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
            let bound = status
                .lines()
                .find_map(|line| line.strip_prefix("CapBnd:"))?;
            Some(bound.trim().to_string())
        })
        .collect();
    assert!(!bounds.is_empty(), "no running instance");
    assert!(
        bounds
            .iter()
            .all(|bound| bound.trim_start_matches('0').is_empty()),
        "{bounds:?}"
    );
    drop(helper);
}

#[test]
#[ignore = "needs the account service: linux-root job (perm-linux-helper)"]
fn perm_task_linux_helper_outdated_reported() {
    let task = helper_task();
    set_up(&task);
    let mut stream = UnixStream::connect(HELPER_SOCKET).expect("connect");
    stream.set_read_timeout(Some(WAIT)).expect("timeout");
    let older = HelperRequest::Hello {
        protocol: HELPER_PROTOCOL + 1,
        se_version: "9.9.9".into(),
    };
    write_frame(&mut stream, &older).expect("hello");
    let reply: HelperReply = read_frame(&mut stream).expect("reply").expect("a frame");
    assert!(
        matches!(
            reply,
            HelperReply::Error {
                code: HelperErrorCode::Outdated,
                ..
            }
        ),
        "{reply:?}"
    );
    // A service whose socket no longer answers needs an update.
    assert!(sudo(&["systemctl", "stop", SOCKET_UNIT]));
    let status = exec_helper_client::helper_status();
    let choices = exec_remote_account::account_choices();
    assert!(sudo(&["systemctl", "start", SOCKET_UNIT]));
    assert_eq!(status.state, HelperState::Outdated, "{status:?}");
    let remote = &choices.choices[1];
    assert_eq!(remote.reason_code, Some(AccountReason::HelperOutdated));
    assert!(remote
        .next_step
        .as_deref()
        .is_some_and(|step| step.contains("update")));
}

#[test]
#[ignore = "needs the account service: linux-root job (perm-linux-helper)"]
fn perm_task_linux_helper_remove_cleans_units() {
    let task = helper_task();
    set_up(&task);
    let (_worker, supervisor) = UnixStream::pair().expect("socket pair");
    let helper = session(&supervisor);
    let unit = helper.name().to_string();
    let remove = |delete_remote_account| ExecAccountsAction::Remove {
        delete_remote_account,
    };
    let report = administer(&task, &remove(false));
    for path in [
        SOCKET_UNIT_FILE,
        SERVICE_UNIT_FILE,
        CONFIG_FILE,
        PAM_FILE,
        PROGRAM_COPY,
        HELPER_SOCKET,
    ] {
        assert!(
            std::fs::symlink_metadata(path).is_err(),
            "{path} remains: {report}"
        );
    }
    let connection = Connection::system().expect("system bus");
    assert!(
        wait_inactive(&connection, &unit),
        "{unit} outlived the removal"
    );
    assert!(
        exec_account::resolve(REMOTE_ACCOUNT_NAME).is_ok(),
        "{report}"
    );
    drop(helper);
    let report = administer(&task, &remove(true));
    assert!(
        exec_account::resolve(REMOTE_ACCOUNT_NAME).is_err(),
        "{report}"
    );
}

#[test]
fn linux_helper_setup_modes_round_trip_and_sudo_command() {
    let actions = [
        ExecAccountsAction::Setup {
            remote_account: true,
            allow: vec!["deploy".into()],
        },
        ExecAccountsAction::Allow {
            account: "deploy".into(),
        },
        ExecAccountsAction::Disallow {
            account: "deploy".into(),
        },
        ExecAccountsAction::Update,
        ExecAccountsAction::Remove {
            delete_remote_account: true,
        },
    ];
    for action in actions {
        let arguments: Vec<OsString> = mode_arguments(&action, 1000)
            .into_iter()
            .map(OsString::from)
            .collect();
        assert_eq!(parse_mode(&arguments).unwrap(), (action.clone(), 1000));
    }
    let setup = ExecAccountsAction::Setup {
        remote_account: true,
        allow: Vec::new(),
    };
    assert_eq!(
        sudo_command(Path::new("/home/a b/.local/opt/smart-explorer/se"), &setup),
        "sudo '/home/a b/.local/opt/smart-explorer/se' share exec accounts setup \
         --remote-account --yes"
    );
    let refused: [&[&str]; 4] = [
        &["--exec-accounts-setup", "--for-uid", "1000", "allow"],
        &["--exec-accounts-setup", "setup"],
        &["--exec-accounts-setup", "--for-uid", "1000", "shell"],
        &[
            "--exec-accounts-remove",
            "--for-uid",
            "1000",
            "--allow",
            "x",
        ],
    ];
    for words in refused {
        let arguments: Vec<OsString> = words.iter().copied().map(OsString::from).collect();
        assert!(parse_mode(&arguments).is_err(), "{arguments:?}");
    }
    assert!(socket_unit_text().contains("Accept=yes\n"));
    assert!(service_unit_text().contains("CapabilityBoundingSet=\n"));
    assert!(service_unit_text().contains(&format!("ExecStart={PROGRAM_COPY} {HELPER_MODE}\n")));
}

/// R15c: one user starts more account sessions at once than the former 8
/// per connection; each runs until its connection ends.
#[test]
#[ignore = "needs the account service: linux-root job (perm-linux-helper)"]
fn perm_task_linux_helper_many_sessions_at_once() {
    let task = helper_task();
    set_up(&task);
    let connection = Connection::system().expect("system bus");
    let mut held = Vec::new();
    for _ in 0..10 {
        let (_worker, supervisor) = UnixStream::pair().expect("socket pair");
        let helper = session(&supervisor);
        held.push((helper, supervisor, _worker));
    }
    let units: Vec<String> = held
        .iter()
        .map(|(helper, ..)| helper.name().to_string())
        .collect();
    for unit in &units {
        assert!(unit_active(&connection, unit), "{unit} does not run");
    }
    drop(held);
    for unit in &units {
        assert!(
            wait_inactive(&connection, unit),
            "{unit} outlived its connection"
        );
    }
}
