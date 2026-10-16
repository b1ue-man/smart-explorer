use super::files::{append, append_rotating, tail_in, Class, ROTATE_AT_BYTES};
use super::*;
use crate::share::{ExecId, ExecLifecycleState, ExecTerminal, ExecTerminalKind};
use std::collections::BTreeMap;
use std::path::Path;

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

fn shell_start() -> ExecStart {
    ExecStart {
        exec_id: ExecId::parse("0a".repeat(16)).unwrap(),
        command: ExecCommand::Shell {
            command: "echo top-secret-token".into(),
        },
        cwd: Some("/srv".into()),
        env: BTreeMap::new(),
        timeout_ms: None,
        max_output_bytes: None,
        terminal: None,
    }
}

fn finished_view(start: &ExecStart) -> ExecJobView {
    ExecJobView {
        exec_id: start.exec_id.clone(),
        peer_device_id: "device-a".into(),
        peer_device_name: "Laptop".into(),
        program: "<shell>".into(),
        command_digest: start.digest().unwrap(),
        state: ExecLifecycleState::Exited,
        policy_revision: 1,
        started_at: Some(100),
        finished_at: Some(103),
        terminal: Some(ExecTerminal {
            exec_id: start.exec_id.clone(),
            kind: ExecTerminalKind::Exited,
            exit_code: Some(0),
            signal: None,
            message: None,
            stdout_bytes: 5,
            stderr_bytes: 0,
            output_truncated: false,
        }),
    }
}

/// Writes `event` the way `record` does, into `directory`.
fn write(directory: &Path, rotate_at: u64, event: &AuditEvent<'_>) {
    append_rotating(directory, event.class(), &line(event), rotate_at).unwrap();
}

fn count(lines: &[String], event: &str) -> usize {
    let needle = format!("\"event\":\"{event}\"");
    lines.iter().filter(|line| line.contains(&needle)).count()
}

#[test]
fn luecken_task_audit_lines_name_peer_and_start_but_never_shell_text() {
    let directory = tempfile::tempdir().unwrap();
    let start = shell_start();
    write(
        directory.path(),
        ROTATE_AT_BYTES,
        &AuditEvent::Started {
            principal: &principal(),
            start: &start,
            account: "standard (root)",
            resolved_program: None,
        },
    );
    write(
        directory.path(),
        ROTATE_AT_BYTES,
        &AuditEvent::Finished {
            principal: &principal(),
            view: &finished_view(&start),
        },
    );
    let lines = tail_in(directory.path(), 10).unwrap();
    assert_eq!(lines.len(), 2);
    let started: Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(started["event"], "started");
    assert_eq!(started["mode"], "shell");
    assert_eq!(started["program"], "<shell>");
    assert_eq!(started["cwd"], "/srv");
    assert_eq!(started["fingerprint"], "fp");
    assert_eq!(started["account"], "standard (root)");
    assert!(!lines[0].contains("top-secret-token"));
    let finished: Value = serde_json::from_str(&lines[1]).unwrap();
    assert_eq!(finished["event"], "finished");
    assert_eq!(finished["result"], "exited");
    assert_eq!(finished["duration_secs"], 3);
    assert_eq!(tail_in(directory.path(), 1).unwrap().len(), 1);
}

#[test]
fn luecken_task_audit_log_rotates_and_keeps_the_previous_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(Class::Executions.names().0);
    let event = json!({ "event": "filler", "pad": "x".repeat(1024) });
    // About 1053 bytes per line: just over one rotation.
    let lines_per_file = ROTATE_AT_BYTES / 1000 + 2;
    for _ in 0..lines_per_file {
        append(directory.path(), Class::Executions, &event).unwrap();
    }
    assert!(directory.path().join(Class::Executions.names().1).exists());
    assert!(std::fs::metadata(&path).unwrap().len() <= ROTATE_AT_BYTES);
    let all = tail_in(directory.path(), usize::MAX).unwrap();
    assert_eq!(all.len() as u64, lines_per_file);
}

#[test]
fn luecken_task_ten_thousand_rejections_never_evict_a_start_or_finish_entry() {
    let directory = tempfile::tempdir().unwrap();
    let start = shell_start();
    let view = finished_view(&start);
    for _ in 0..3 {
        write(
            directory.path(),
            ROTATE_AT_BYTES,
            &AuditEvent::Started {
                principal: &principal(),
                start: &start,
                account: "standard (root)",
                resolved_program: None,
            },
        );
        write(
            directory.path(),
            ROTATE_AT_BYTES,
            &AuditEvent::Finished {
                principal: &principal(),
                view: &view,
            },
        );
    }
    write(
        directory.path(),
        ROTATE_AT_BYTES,
        &AuditEvent::StartRefused {
            principal: &principal(),
            start: &start,
            reason: "program not permitted",
        },
    );
    // A small rotation size makes the flood rotate its own files many times.
    let small = 64 * 1024;
    for index in 0..10_000 {
        let endpoint = format!("{index:064x}");
        write(
            directory.path(),
            small,
            &AuditEvent::AuthRejected {
                remote_node: &endpoint,
                claimed_device: "device-x",
                reason: "authentication failed",
            },
        );
        if index % 3 == 0 {
            write(
                directory.path(),
                small,
                &AuditEvent::PenaltyRefused {
                    remote_node: &endpoint,
                    protocol: "exec",
                    remaining_secs: 15,
                },
            );
        }
    }
    let lines = tail_in(directory.path(), usize::MAX).unwrap();
    assert_eq!(count(&lines, "started"), 3);
    assert_eq!(count(&lines, "finished"), 3);
    assert_eq!(count(&lines, "start_refused"), 1);
    assert!(directory.path().join(Class::Rejections.names().1).exists());
    assert!(!directory.path().join(Class::Executions.names().1).exists());
    // The newest rejection is still there, the oldest ones were rotated out.
    assert!(lines
        .iter()
        .any(|line| line.contains(&format!("{:064x}", 9_999))));
    assert!(!lines
        .iter()
        .any(|line| line.contains(&format!("{:064x}", 0))));
}

#[test]
fn luecken_task_tail_lists_both_classes_oldest_first() {
    let directory = tempfile::tempdir().unwrap();
    append(
        directory.path(),
        Class::Rejections,
        &json!({ "event": "auth_rejected", "at": "2026-10-16T10:00:02Z" }),
    )
    .unwrap();
    append(
        directory.path(),
        Class::Executions,
        &json!({ "event": "started", "at": "2026-10-16T10:00:01Z" }),
    )
    .unwrap();
    append(
        directory.path(),
        Class::Executions,
        &json!({ "event": "finished", "at": "2026-10-16T10:00:03Z" }),
    )
    .unwrap();
    let lines = tail_in(directory.path(), 10).unwrap();
    assert_eq!(count(&lines[..1], "started"), 1);
    assert_eq!(count(&lines[1..2], "auth_rejected"), 1);
    assert_eq!(count(&lines[2..], "finished"), 1);
    assert_eq!(tail_in(directory.path(), 2).unwrap().len(), 2);
}

#[test]
fn luecken_task_peer_text_is_cut_and_cleaned() {
    let mut name = String::from("Evil\u{9b}[2J\u{202e}name");
    name.push_str(&"x".repeat(10_000));
    let mut start = shell_start();
    start.cwd = Some("y".repeat(10_000));
    let mut who = principal();
    who.device_name = name.clone();
    let value = line(&AuditEvent::StartRefused {
        principal: &who,
        start: &start,
        reason: &name,
    });
    let encoded = serde_json::to_string(&value).unwrap();
    assert!(encoded.len() < 16 * 1024, "{} bytes", encoded.len());
    assert!(!encoded.contains('\u{9b}') && !encoded.contains('\u{202e}'));
    let shown = value["device_name"].as_str().unwrap();
    assert_eq!(shown.chars().count(), text::MAX_FIELD_CHARS + 1);
    assert!(shown.starts_with("Evil?[2J?name") && shown.ends_with('…'));
    assert_eq!(value["cwd"].as_str().unwrap().chars().count(), 257);
    let claimed = line(&AuditEvent::AuthRejected {
        remote_node: "node",
        claimed_device: &name,
        reason: "authentication failed",
    });
    assert_eq!(
        claimed["claimed_device"].as_str().unwrap().chars().count(),
        text::MAX_FIELD_CHARS + 1
    );
}

#[test]
fn luecken_task_a_resolved_program_is_named_in_the_started_entry() {
    let start = shell_start();
    let value = line(&AuditEvent::Started {
        principal: &principal(),
        start: &start,
        account: "deploy (uid 1001)",
        resolved_program: Some("/usr/bin/uptime"),
    });
    assert_eq!(value["resolved_program"], "/usr/bin/uptime");
    assert_eq!(value["account"], "deploy (uid 1001)");
}

#[cfg(unix)]
#[test]
fn luecken_task_audit_files_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    for class in Class::ALL {
        append(directory.path(), class, &json!({ "event": "probe" })).unwrap();
        let mode = std::fs::metadata(directory.path().join(class.names().0))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "{class:?}");
    }
}
