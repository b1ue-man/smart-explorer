//! PRM-M4: every entry names kind, account, capability and confirmation;
//! confirmations and ended grants are entries of their own; shell text never
//! reaches the log; files and rotation are those of the Exec audit log.
use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::*;
use crate::share::exec_audit::for_tests::{append_event, entries, file_names, line_of, tail};
use crate::share::exec_audit::{AuditEvent, StartAccess, ROTATE_AT};
use crate::share::{
    ExecCommand, ExecId, ExecJobView, ExecLifecycleState, ExecTerminal, ExecTerminalKind,
};

const SECRET: &str = "echo top-secret-token";

fn principal() -> ExecPrincipal {
    ExecPrincipal {
        relation_kind: "direct".into(),
        relation_id: "lookup".into(),
        device_id: "device-a".into(),
        device_name: "Laptop".into(),
        public_key: "key".into(),
        fingerprint: "fp".into(),
        node_id: "node".into(),
    }
}

fn start(command: ExecCommand) -> ExecStart {
    ExecStart {
        exec_id: ExecId::parse("0b".repeat(16)).unwrap(),
        command,
        cwd: Some("/srv".into()),
        env: BTreeMap::new(),
        timeout_ms: None,
        max_output_bytes: None,
        terminal: None,
    }
}

fn shell() -> ExecStart {
    start(ExecCommand::Shell {
        command: SECRET.into(),
    })
}

fn entry<'a>(
    kind: AccessKind<'a>,
    principal: &'a ProvenPrincipal,
    outcome: Option<AccessEvent>,
    approval: Option<ApprovalResult>,
) -> AccessEntry<'a> {
    AccessEntry {
        kind,
        principal,
        outcome,
        capability: Capability::ExecCommands,
        approval,
        approval_id: Some("ab".repeat(16).leak()),
        account_profile: Some("remote"),
        account: Some("se-remote"),
        reason: None,
    }
}

fn finished(start: &ExecStart) -> ExecJobView {
    ExecJobView {
        exec_id: start.exec_id.clone(),
        peer_device_id: "device-a".into(),
        peer_device_name: "Laptop".into(),
        program: "<shell>".into(),
        command_digest: start.digest().unwrap(),
        state: ExecLifecycleState::Exited,
        policy_revision: 2,
        started_at: Some(100),
        finished_at: Some(101),
        terminal: Some(ExecTerminal {
            exec_id: start.exec_id.clone(),
            kind: ExecTerminalKind::Exited,
            exit_code: Some(0),
            signal: None,
            message: None,
            stdout_bytes: 1,
            stderr_bytes: 0,
            output_truncated: false,
        }),
    }
}

fn access() -> StartAccess {
    StartAccess {
        capability: Capability::ExecCommands,
        account_profile: "remote",
        approval: "once",
    }
}

#[test]
fn perm_task_audit_fields_exec() {
    let start = shell();
    let who = principal();
    let proven = ProvenPrincipal::from(&who);
    let exec = AccessKind::Exec {
        start: Some(&start),
    };
    let requested = access_line(&entry(
        exec,
        &proven,
        Some(AccessEvent::ApprovalRequested),
        None,
    ));
    assert_eq!(requested["event"], "approval_requested");
    assert_eq!(requested["kind"], "exec");
    assert_eq!(requested["capability"], "exec_commands");
    assert_eq!(requested["account_profile"], "remote");
    assert_eq!(requested["account"], "se-remote");
    assert_eq!(requested["mode"], "shell");
    assert_eq!(requested["digest"], json!(start.digest().unwrap()));
    assert_eq!(
        (&requested["device_id"], &requested["fingerprint"]),
        (&json!("device-a"), &json!("fp"))
    );
    assert!(requested.get("approval").is_none());
    let exec = AccessKind::Exec {
        start: Some(&start),
    };
    let granted = access_line(&entry(
        exec,
        &proven,
        Some(AccessEvent::ApprovalGranted),
        Some(ApprovalResult::Once),
    ));
    assert_eq!(granted["approval"], "once");
    // The start and its end carry how the access rights let it run.
    let started = line_of(
        &AuditEvent::Started {
            principal: &who,
            start: &start,
            account: "se-remote (uid 1002)",
            resolved_program: None,
        },
        Some(&access()),
    );
    for (key, value) in [
        ("event", "started"),
        ("kind", "exec"),
        ("capability", "exec_commands"),
        ("account_profile", "remote"),
        ("approval", "once"),
        ("account", "se-remote (uid 1002)"),
    ] {
        assert_eq!(started[key], value, "{key}");
    }
    let ended = line_of(
        &AuditEvent::Finished {
            principal: &who,
            view: &finished(&start),
        },
        Some(&access()),
    );
    assert_eq!(
        (&ended["kind"], &ended["approval"]),
        (&json!("exec"), &json!("once"))
    );
    let refused = line_of(
        &AuditEvent::StartRefused {
            principal: &who,
            start: &start,
            reason: "program not permitted",
        },
        None,
    );
    assert_eq!(
        (&refused["kind"], &refused["capability"]),
        (&json!("exec"), &json!("exec_commands"))
    );
    // A screen session of S4 is an entry of the same log with its own kind.
    let screen = AccessKind::Screen {
        mode: "view",
        event: "session_started",
    };
    let watched = access_line(&entry(
        screen,
        &proven,
        None,
        Some(ApprovalResult::NotRequired),
    ));
    assert_eq!(
        (&watched["event"], &watched["kind"], &watched["mode"]),
        (&json!("session_started"), &json!("screen"), &json!("view"))
    );
    // As the GUIs and `share.execLog` show them.
    let directory = tempfile::tempdir().unwrap();
    for (seconds, mut line) in [(1, started), (2, granted), (3, watched)] {
        line["at"] = json!(format!("2026-10-17T10:00:0{seconds}Z"));
        exec_audit::append_access(directory.path(), &line, ROTATE_AT).unwrap();
    }
    let old = json!({"event": "started", "at": "2026-10-17T09:00:00Z", "device_id": "device-a", "mode": "argv", "program": "uptime", "account": "standard (alice)"});
    exec_audit::append_access(directory.path(), &old, ROTATE_AT).unwrap();
    let shown = entries(directory.path(), 10);
    assert_eq!(shown.len(), 4);
    assert_eq!(
        (shown[0].kind.as_str(), shown[0].title.as_str()),
        ("screen", "Bildschirm: session_started")
    );
    assert_eq!(
        (shown[1].title.as_str(), shown[1].approval.as_str()),
        ("Rückfrage erlaubt", "once")
    );
    assert!(
        shown[1].detail.contains("Recht: Befehle")
            && shown[1].detail.contains("Rückfrage: diesmal erlaubt")
    );
    assert_eq!(
        (shown[2].kind.as_str(), shown[2].account.as_str()),
        ("exec", "se-remote (uid 1002)")
    );
    assert!(shown[2].detail.contains("Rückfrage: diesmal erlaubt"));
    // A line from before the access rights is an Exec line without answer.
    assert_eq!(
        (shown[3].kind.as_str(), shown[3].approval.as_str()),
        ("exec", "")
    );
    let bridge = serde_json::to_value(&shown[1]).unwrap();
    let keys: Vec<&str> = bridge
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    for key in [
        "at", "event", "title", "device", "deviceId", "detail", "kind", "account", "approval",
    ] {
        assert!(keys.contains(&key), "share.execLog lacks {key}");
    }
}

#[test]
fn perm_task_audit_no_shell_text() {
    let start = shell();
    let who = principal();
    let proven = ProvenPrincipal::from(&who);
    let events = [
        (AccessEvent::ApprovalRequested, None),
        (AccessEvent::ApprovalGranted, Some(ApprovalResult::Always)),
        (AccessEvent::ApprovalDenied, Some(ApprovalResult::Denied)),
        (
            AccessEvent::ApprovalCancelled,
            Some(ApprovalResult::Cancelled),
        ),
        (
            AccessEvent::ApprovalUnavailable,
            Some(ApprovalResult::Unavailable),
        ),
        (AccessEvent::GrantExpired, Some(ApprovalResult::Cancelled)),
    ];
    let mut lines: Vec<Value> = events
        .into_iter()
        .map(|(event, result)| {
            let kind = AccessKind::Exec {
                start: Some(&start),
            };
            access_line(&entry(kind, &proven, Some(event), result))
        })
        .collect();
    lines.push(line_of(
        &AuditEvent::Started {
            principal: &who,
            start: &start,
            account: "standard (alice)",
            resolved_program: None,
        },
        Some(&access()),
    ));
    lines.push(line_of(
        &AuditEvent::StartRefused {
            principal: &who,
            start: &start,
            reason: "approval_denied",
        },
        None,
    ));
    for line in &lines {
        let text = serde_json::to_string(line).unwrap();
        assert!(!text.contains("top-secret-token"), "{text}");
        assert_eq!(line["program"], "<shell>");
    }
    let directory = tempfile::tempdir().unwrap();
    for line in &lines {
        exec_audit::append_access(directory.path(), line, ROTATE_AT).unwrap();
    }
    for shown in entries(directory.path(), 20) {
        assert!(!format!("{shown:?}").contains("top-secret-token"));
    }
    // Only the request the host user sees names the command (cut, never logged).
    let command = crate::share::access_approval::ApprovalCommand::of(&start.command);
    assert!(
        matches!(command, crate::share::access_approval::ApprovalCommand::Shell { text } if text == SECRET)
    );
}

#[test]
fn perm_task_audit_rotation_unchanged() {
    assert_eq!(ROTATE_AT, 4 * 1024 * 1024);
    assert_eq!(
        file_names(),
        [
            ("exec-audit.log", "exec-audit.log.1"),
            ("exec-audit-rejected.log", "exec-audit-rejected.log.1")
        ]
    );
    let directory = tempfile::tempdir().unwrap();
    let who = principal();
    let proven = ProvenPrincipal::from(&who);
    let start = start(ExecCommand::Login);
    let line = access_line(&entry(
        AccessKind::Exec {
            start: Some(&start),
        },
        &proven,
        Some(AccessEvent::ApprovalRequested),
        None,
    ));
    let size = serde_json::to_string(&line).unwrap().len() as u64 + 1;
    // Access lines share the file of authorized devices and rotate with it.
    let small = size * 3;
    for _ in 0..4 {
        exec_audit::append_access(directory.path(), &line, small).unwrap();
    }
    let event = AuditEvent::AuthRejected {
        remote_node: "node-x",
        claimed_device: "device-x",
        reason: "authentication failed",
    };
    append_event(directory.path(), &event, small);
    let [(current, rotated), (rejected, _)] = file_names();
    let path = |name: &str| directory.path().join(name);
    assert!(path(rotated).exists());
    assert!(std::fs::metadata(path(current)).unwrap().len() <= small);
    assert!(path(rejected).exists());
    let all = tail(directory.path(), usize::MAX);
    let requested = all
        .iter()
        .filter(|line| line.contains("approval_requested"))
        .count();
    // Three before the rotation are kept in `.1`, one after it in the file.
    assert_eq!(requested, 4);
    assert_eq!(
        all.iter()
            .filter(|line| line.contains("auth_rejected"))
            .count(),
        1
    );
}
