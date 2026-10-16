use std::collections::BTreeMap;

use serde::Deserialize;

use super::program_path::{self, ProgramSearch};
use super::*;
use crate::share::ExecId;

fn search(directory: &Path) -> ProgramSearch {
    ProgramSearch {
        directories: vec![directory.to_path_buf()],
        suffixes: &[],
    }
}

fn argv(program: &str, env: &[(&str, &str)]) -> ExecStart {
    ExecStart {
        exec_id: ExecId::parse("0b".repeat(16)).unwrap(),
        command: ExecCommand::Argv {
            program: program.into(),
            args: vec!["--any-argument".into()],
        },
        cwd: None,
        env: env
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect::<BTreeMap<_, _>>(),
        timeout_ms: None,
        max_output_bytes: None,
        terminal: None,
    }
}

fn listed(programs: &[&str]) -> ExecGrant {
    ExecGrant {
        enabled: true,
        policy_revision: 4,
        programs: programs.iter().map(|program| program.to_string()).collect(),
        ..ExecGrant::default()
    }
}

fn touch(directory: &Path, name: &str) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, b"#!/bin/sh\n").unwrap();
    path
}

#[test]
fn luecken_task_program_list_refuses_loader_path_and_pager_variables() {
    let trusted = tempfile::tempdir().unwrap();
    touch(trusted.path(), "uptime");
    let grant = listed(&["uptime"]);
    let search = search(trusted.path());
    for name in ["LD_PRELOAD", "PATH", "GIT_PAGER", "LD_LIBRARY_PATH", "lang"] {
        let error = grant
            .authorize_start(&argv("uptime", &[(name, "x")]), &search)
            .unwrap_err();
        assert!(error.contains(name), "{error}");
    }
    // A pager variable hides behind a permitted one too: names are exact.
    for (name, value) in [("LANG", "/tmp/locale"), ("TERM", "x y"), ("LC_ALL", "a;b")] {
        let error = grant
            .authorize_start(&argv("uptime", &[(name, value)]), &search)
            .unwrap_err();
        assert!(error.contains("plain locale or terminal name"), "{error}");
    }
    let accepted = [
        ("LANG", "en_US.UTF-8"),
        ("LC_ALL", "C"),
        ("TERM", "xterm-256color"),
    ];
    let path = grant
        .authorize_start(&argv("uptime", &accepted), &search)
        .unwrap()
        .unwrap();
    assert_eq!(path, trusted.path().join("uptime"));
    // LC_ must be followed by a locale category name.
    assert!(grant
        .authorize_start(&argv("uptime", &[("LC_", "C")]), &search)
        .is_err());
    assert!(grant
        .authorize_start(&argv("uptime", &[("LC_all", "C")]), &search)
        .is_err());
}

#[test]
fn luecken_task_full_shell_grants_keep_their_environment_and_lookup() {
    let grant = ExecGrant {
        enabled: true,
        ..ExecGrant::default()
    };
    let nowhere = ProgramSearch {
        directories: Vec::new(),
        suffixes: &[],
    };
    let start = argv(
        "anything",
        &[("PATH", "/tmp/x"), ("LD_PRELOAD", "/tmp/y.so")],
    );
    assert_eq!(grant.authorize_start(&start, &nowhere), Ok(None));
    assert!(!grant.is_restricted());
    let account = ExecGrant {
        run_as: ExecRunAs::Account {
            name: "deploy".into(),
        },
        ..grant
    };
    assert!(account.is_restricted());
    assert_eq!(account.authorize_start(&start, &nowhere), Ok(None));
}

#[test]
fn luecken_task_program_resolution_ignores_the_callers_path() {
    let trusted = tempfile::tempdir().unwrap();
    let hostile = tempfile::tempdir().unwrap();
    let trusted_tool = touch(trusted.path(), "tool");
    let hostile_tool = touch(hostile.path(), "tool");
    touch(hostile.path(), "other");
    let grant = listed(&["tool"]);
    let search = search(trusted.path());

    // The bare name resolves in the fixed path only.
    let resolved = grant
        .authorize_start(&argv("tool", &[]), &search)
        .unwrap()
        .unwrap();
    assert_eq!(resolved, trusted_tool);
    // A program that exists only where a caller's PATH would point is unknown.
    let error = grant
        .authorize_start(&argv("other", &[]), &search)
        .unwrap_err();
    assert!(error.contains("fixed search path"), "{error}");
    // Another file under the listed name is a different program.
    let error = grant
        .authorize_start(&argv(hostile_tool.to_str().unwrap(), &[]), &search)
        .unwrap_err();
    assert!(error.contains("not one of them"), "{error}");
    // A relative path depends on the working directory and is never used.
    let error = program_path::resolve("./tool", &search).unwrap_err();
    assert!(error.contains("not an absolute path"), "{error}");
    // The caller's PATH can neither be sent nor consulted.
    let error = grant
        .authorize_start(
            &argv("tool", &[("PATH", hostile.path().to_str().unwrap())]),
            &search,
        )
        .unwrap_err();
    assert!(error.contains("\"PATH\""), "{error}");
}

#[test]
fn luecken_task_program_list_refuses_shell_commands_and_terminals() {
    let grant = listed(&["uptime"]);
    let nowhere = ProgramSearch {
        directories: Vec::new(),
        suffixes: &[],
    };
    let mut start = argv("uptime", &[]);
    start.command = ExecCommand::Shell {
        command: "uptime".into(),
    };
    assert!(grant
        .authorize_start(&start, &nowhere)
        .unwrap_err()
        .contains("no shell commands"));
    start.command = ExecCommand::Login;
    assert!(grant
        .authorize_start(&start, &nowhere)
        .unwrap_err()
        .contains("no terminal"));
}

#[test]
fn luecken_task_allowed_programs_are_bare_names_or_absolute_paths() {
    let settings = |programs: &[&str]| ExecGrantSettings {
        run_as: ExecRunAs::Standard,
        programs: programs.iter().map(|program| program.to_string()).collect(),
    };
    assert!(settings(&["uptime"]).validate().is_ok());
    let absolute = std::env::temp_dir().join("tool");
    assert!(settings(&[absolute.to_str().unwrap()]).validate().is_ok());
    for relative in ["./tool", "bin/tool", "../tool"] {
        let error = settings(&[relative]).validate().unwrap_err();
        assert!(error.contains("bare name or an absolute path"), "{error}");
    }
    assert!(settings(&[" "]).validate().is_err());
    assert!(settings(&["a\0b"]).validate().is_err());
}

#[cfg(unix)]
#[test]
fn luecken_task_a_listed_applet_does_not_open_the_other_applets_of_its_binary() {
    use std::os::unix::fs::symlink;
    let trusted = tempfile::tempdir().unwrap();
    let multi = touch(trusted.path(), "multi");
    symlink(&multi, trusted.path().join("ls")).unwrap();
    symlink(&multi, trusted.path().join("sh")).unwrap();
    let grant = listed(&["ls"]);
    let search = search(trusted.path());
    let path = grant
        .authorize_start(&argv("ls", &[]), &search)
        .unwrap()
        .unwrap();
    // The job starts under the applet's own name, not as the binary.
    assert_eq!(path, trusted.path().join("ls"));
    for other in ["sh", "multi"] {
        let error = grant
            .authorize_start(&argv(other, &[]), &search)
            .unwrap_err();
        assert!(error.contains("not one of them"), "{other}: {error}");
    }
}

/// The field list of `ExecGrant` in 0.5.180
/// (`git show HEAD:native/src/share/core/exec_policy.rs`): five fields, no
/// `deny_unknown_fields`, every field defaulted.
#[derive(Debug, Deserialize)]
struct HeadExecGrant {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    policy_revision: u64,
    #[serde(default)]
    changed_at: i64,
    #[serde(default)]
    source_request_id: Option<DirectRequestId>,
    #[serde(default)]
    source_decision_revision: Option<u64>,
}

fn stored(grant: &ExecGrant) -> serde_json::Value {
    serde_json::to_value(grant).unwrap()
}

#[test]
fn luecken_task_head_reads_a_program_list_grant_as_switched_off() {
    let grant = listed(&["/usr/bin/uptime"]);
    let value = stored(&grant);
    let head: HeadExecGrant = serde_json::from_value(value.clone()).unwrap();
    assert!(!head.enabled, "0.5.180 must read the narrowed grant as off");
    assert_eq!(head.policy_revision, 4);
    assert_eq!(head.changed_at, 0);
    assert!(head.source_request_id.is_none() && head.source_decision_revision.is_none());
    // This version reads its own output unchanged.
    let back: ExecGrant = serde_json::from_value(value).unwrap();
    assert_eq!(back, grant);
    assert!(back.enabled);
}

#[test]
fn luecken_task_head_reads_another_account_grant_as_switched_off() {
    let grant = ExecGrant {
        enabled: true,
        policy_revision: 9,
        run_as: ExecRunAs::Account {
            name: "deploy".into(),
        },
        ..ExecGrant::default()
    };
    let value = stored(&grant);
    let head: HeadExecGrant = serde_json::from_value(value.clone()).unwrap();
    assert!(!head.enabled);
    assert_eq!(head.policy_revision, 9);
    let back: ExecGrant = serde_json::from_value(value).unwrap();
    assert_eq!(back, grant);
}

#[test]
fn luecken_task_unrestricted_and_switched_off_grants_are_stored_as_before() {
    let full = ExecGrant {
        enabled: true,
        policy_revision: 3,
        ..ExecGrant::default()
    };
    let value = stored(&full);
    assert_eq!(value["enabled"], true);
    assert!(value.get("restricted_enabled").is_none());
    let head: HeadExecGrant = serde_json::from_value(value).unwrap();
    assert!(head.enabled);

    let off = ExecGrant {
        enabled: false,
        programs: vec!["uptime".into()],
        ..ExecGrant::default()
    };
    let value = stored(&off);
    assert_eq!(value["enabled"], false);
    assert!(value.get("restricted_enabled").is_none());
    let back: ExecGrant = serde_json::from_value(value).unwrap();
    assert_eq!(back, off);
}

#[test]
fn luecken_task_a_stray_restricted_flag_never_switches_a_full_shell_on() {
    let value = serde_json::json!({ "enabled": false, "restricted_enabled": true });
    let grant: ExecGrant = serde_json::from_value(value).unwrap();
    assert!(!grant.enabled);
}

#[test]
fn luecken_task_a_profile_written_by_0_5_180_keeps_rights_and_revision_when_saved_again() {
    let head_format = serde_json::json!({
        "enabled": true,
        "policy_revision": 6,
        "changed_at": 1_700_000_000,
        "source_request_id": null,
        "source_decision_revision": 2
    });
    let grant: ExecGrant = serde_json::from_value(head_format).unwrap();
    assert!(grant.enabled);
    assert_eq!(grant.policy_revision, 6);
    assert_eq!(grant.run_as, ExecRunAs::Standard);
    assert!(grant.programs.is_empty() && !grant.is_restricted());
    // The first save writes the same rights and revision, and 0.5.180 reads
    // that output exactly as before.
    let saved = stored(&grant);
    let again: ExecGrant = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(again, grant);
    let head: HeadExecGrant = serde_json::from_value(saved).unwrap();
    assert!(head.enabled);
    assert_eq!(head.policy_revision, 6);
    assert_eq!(head.source_decision_revision, Some(2));
}

#[test]
fn luecken_task_changing_account_or_programs_advances_the_revision_and_switching_keeps_them() {
    let mut grant = listed(&["uptime"]);
    let before = grant.policy_revision;
    let settings = ExecGrantSettings {
        run_as: ExecRunAs::Account {
            name: "deploy".into(),
        },
        programs: vec!["date".into()],
    };
    grant.set_runtime_policy(true, Some(&settings), 50).unwrap();
    assert_eq!(grant.policy_revision, before + 1);
    assert_eq!(grant.programs, settings.programs);
    // Switching off and on without settings (GUI, Android, old IPC) keeps both.
    grant.set_runtime_policy(false, None, 51).unwrap();
    grant.set_runtime_policy(true, None, 52).unwrap();
    assert_eq!(grant.policy_revision, before + 3);
    assert_eq!(grant.run_as, settings.run_as);
    assert_eq!(grant.programs, settings.programs);
    // A change of identity or of the base decision switches a narrowed grant off.
    grant.reset_for_identity_change(60);
    assert!(!grant.enabled);
    assert_eq!(grant.programs, settings.programs);
    grant.set_runtime_policy(true, None, 61).unwrap();
    grant.disable_without_decision(62);
    assert!(!grant.enabled);
}

#[test]
fn luecken_task_another_account_is_refused_on_hosts_that_cannot_switch_to_it() {
    let account = ExecGrantSettings {
        run_as: ExecRunAs::Account {
            name: "deploy".into(),
        },
        programs: Vec::new(),
    };
    // Windows, Android and a Linux worker without root: refused with the
    // reason; a root worker on Linux accepts the name (resolution happens at
    // the start).
    let result = account.validate_for_this_host();
    assert_eq!(result.is_ok(), crate::share::exec_run_as_supported());
    if let Err(reason) = result {
        assert!(reason.contains("Linux worker running as root"), "{reason}");
    }
    assert!(ExecGrantSettings::default()
        .validate_for_this_host()
        .is_ok());
    let bad = ExecGrantSettings {
        run_as: ExecRunAs::Account { name: "-rf".into() },
        programs: Vec::new(),
    };
    assert!(bad.validate_for_this_host().is_err());
}
