//! `share.exec` over the Exec stream without a worker (fix of the old
//! one-shot path, which every host refused): the start built from the
//! arguments, results and refusals as the app reads them, and the task
//! messages while a start waits for the peer's user and once it runs.
use serde_json::json;

use super::super::{exec_start, profile_name};
use super::{
    account_line, connecting_line, failure_error, result_json, running_line, terminal_error,
    waiting_line, Output,
};
use crate::daemon::ExecIpcFailure;
use crate::share::access_grant::CapabilityDecision;
use crate::share::exec_access_view::{ApprovalSurface, ExecAccessView};
use crate::share::{
    ExecCommand, ExecId, ExecTerminal, ExecTerminalKind, PeerOpenTarget, ShareProfiles,
};

fn terminal(kind: ExecTerminalKind) -> ExecTerminal {
    ExecTerminal {
        exec_id: ExecId::parse("00".repeat(16)).expect("id"),
        kind,
        exit_code: Some(0),
        signal: None,
        message: None,
        stdout_bytes: 0,
        stderr_bytes: 0,
        output_truncated: false,
    }
}

fn failure(kind: &str, code: &str) -> ExecIpcFailure {
    ExecIpcFailure {
        kind: kind.into(),
        code: code.into(),
        message: "refused".into(),
        start_may_have_been_sent: true,
    }
}

#[test]
fn perm_task_bridge_exec_stream_start_from_arguments() {
    let (target, start, command) = exec_start(&json!({
        "location": "share://direct/c-laptop",
        "command": "  uptime -p ",
        "shell": false,
        "timeoutSecs": 15,
    }))
    .expect("argv start");
    assert_eq!(
        target,
        PeerOpenTarget::Direct {
            contact_id: "c-laptop".into()
        }
    );
    assert_eq!(command, "uptime -p");
    assert_eq!(
        start.command,
        ExecCommand::Argv {
            program: "uptime".into(),
            args: vec!["-p".into()],
        }
    );
    assert_eq!(start.cwd, None);
    assert!(start.env.is_empty());
    assert_eq!(start.timeout_ms, Some(15_000));
    assert_eq!(start.max_output_bytes, Some(1024 * 1024));
    assert_eq!(start.terminal, None);
    start.validate().expect("valid start");

    let (room, shell, _) = exec_start(&json!({
        "location": "share://room/r1/d2/",
        "command": "echo $HOME | wc -c",
        "shell": true,
        "timeoutSecs": 60,
    }))
    .expect("shell start");
    assert_eq!(
        room,
        PeerOpenTarget::RoomDevice {
            room_id: "r1".into(),
            device_id: "d2".into(),
        }
    );
    assert_eq!(
        shell.command,
        ExecCommand::Shell {
            command: "echo $HOME | wc -c".into()
        }
    );
    assert_ne!(start.exec_id, shell.exec_id);

    for (refused, why) in [
        (
            json!({ "location": "share://direct/c-laptop/Dokumente", "command": "ls", "timeoutSecs": 5 }),
            "a folder is no working directory of the host",
        ),
        (
            json!({ "location": "/sdcard", "command": "ls", "timeoutSecs": 5 }),
            "not a Share device",
        ),
        (
            json!({ "location": "share://direct/c-laptop", "command": "   ", "timeoutSecs": 5 }),
            "empty command",
        ),
        (
            json!({ "location": "share://direct/c-laptop", "command": "\"\"", "timeoutSecs": 5 }),
            "empty program",
        ),
        (
            json!({ "location": "share://direct/c-laptop", "command": "echo 'open", "timeoutSecs": 5 }),
            "open quote",
        ),
        (
            json!({ "location": "share://direct/c-laptop", "command": "ls", "timeoutSecs": 0 }),
            "time limit",
        ),
    ] {
        assert_eq!(
            exec_start(&refused).expect_err(why).kind,
            "invalid",
            "{why}"
        );
    }
    assert_eq!(
        profile_name(
            &ShareProfiles::default(),
            &PeerOpenTarget::Direct {
                contact_id: "c-laptop".into()
            }
        ),
        None
    );
}

#[test]
fn perm_task_bridge_exec_stream_results_and_refusals() {
    let mut output = Output::default();
    output.push(false, b"up 3 days\n");
    output.push(true, b"warn\n");
    let header = "Laptop: als se-remote, mit Rückfrage";
    assert_eq!(
        result_json(&terminal(ExecTerminalKind::Exited), &output, Some(header)),
        json!({
            "stdout": "up 3 days\n",
            "stderr": "warn\n",
            "exitCode": 0,
            "timedOut": false,
            "truncated": false,
            "signal": null,
            "header": header,
        })
    );
    assert!(terminal_error(&terminal(ExecTerminalKind::Exited), "Laptop").is_none());
    assert!(terminal_error(&terminal(ExecTerminalKind::TimedOut), "Laptop").is_none());
    assert_eq!(
        result_json(&terminal(ExecTerminalKind::TimedOut), &output, None)["timedOut"],
        true
    );

    // stdout and stderr together up to 1 MiB, then marked as cut.
    let mut big = Output::default();
    big.push(false, &vec![b'a'; 1024 * 1024 - 1]);
    big.push(true, b"bc");
    assert_eq!(big.stdout.len() + big.stderr.len(), 1024 * 1024);
    assert_eq!(big.stderr, b"b");
    assert!(big.truncated);

    // Every other end is an error that names the device and the reason.
    let mut revoked = terminal(ExecTerminalKind::Revoked);
    revoked.message = Some("grant removed".into());
    let error = terminal_error(&revoked, "Laptop").expect("revoked");
    assert_eq!(error.kind, "permission");
    assert!(error.message.contains("Laptop"), "{}", error.message);
    assert!(error.message.contains("grant removed"), "{}", error.message);
    for (kind, expected) in [
        (ExecTerminalKind::Cancelled, "conflict"),
        (ExecTerminalKind::Disconnected, "network"),
        (ExecTerminalKind::Failed, "internal"),
    ] {
        assert_eq!(
            terminal_error(&terminal(kind.clone()), "Laptop")
                .expect("error")
                .kind,
            expected,
            "{kind:?}"
        );
    }

    // Refusals keep their code first: ExecDialog.kt shows the German text.
    for code in [
        "permission_denied",
        "right_not_granted",
        "approval_denied",
        "approval_cancelled",
        "approval_unavailable",
        "account_not_allowed",
        "account_helper_unavailable",
        "account_unavailable",
    ] {
        let error = failure_error(&failure("remote", code));
        assert_eq!(error.kind, "permission", "{code}");
        assert_eq!(error.message, format!("{code}: refused"));
    }
    for (kind, code, expected) in [
        ("authentication", "authentication_failed", "permission"),
        ("disconnected", "disconnected", "network"),
        ("protocol", "protocol_error", "network"),
        ("local", "local_ipc_failed", "internal"),
        ("remote", "exec_limit", "conflict"),
    ] {
        assert_eq!(failure_error(&failure(kind, code)).kind, expected, "{kind}");
    }
}

#[test]
fn perm_task_bridge_exec_stream_waiting_and_account_lines() {
    assert_eq!(connecting_line("Laptop"), "Verbinde mit Laptop …");
    assert_eq!(
        waiting_line("Laptop", Some(ApprovalSurface::Notification)),
        "Warte auf Bestätigung am Gerät Laptop …"
    );
    assert_eq!(
        waiting_line("Laptop", Some(ApprovalSurface::Window)),
        "Warte auf Bestätigung am Gerät Laptop …"
    );
    assert_eq!(
        waiting_line("Server", Some(ApprovalSurface::CliOnly)),
        "Warte auf Bestätigung am Gerät Server … (am Gerät nur mit „se share exec approvals“ zu bestätigen)"
    );
    let asks = ExecAccessView {
        account_label: "se-remote".into(),
        account_profile: "remote".into(),
        commands: CapabilityDecision::Ask,
        terminal: CapabilityDecision::Off,
        programs_restricted: false,
        valid_until: None,
    };
    assert_eq!(
        account_line("Laptop", Some(&asks), "anna"),
        "Laptop: als se-remote, mit Rückfrage"
    );
    let allowed = ExecAccessView {
        commands: CapabilityDecision::Allow,
        ..asks.clone()
    };
    assert_eq!(
        account_line("Laptop", Some(&allowed), "anna"),
        "Laptop: als se-remote"
    );
    assert_eq!(
        account_line("Laptop", None, "anna"),
        "Laptop: Konto anna (älteres Gerät, Rechte nicht gemeldet)"
    );
    assert_eq!(
        running_line(Some("Laptop: als anna")),
        "Laptop: als anna · Befehl läuft …"
    );
    assert_eq!(running_line(None), "Befehl läuft …");
}
