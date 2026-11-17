use clap::{CommandFactory, Parser};

use super::{paused_line, validate_source, CompletedInstall};
use crate::cli::Cli;
use crate::daemon::WorkerHandoff;

fn parses(arguments: &str) -> bool {
    Cli::try_parse_from(std::iter::once("se").chain(arguments.split_whitespace())).is_ok()
}

#[test]
fn cli_task_update_parses_check_reinstall_source_and_hidden_completion() {
    for accepted in [
        "update",
        "update --check",
        "update --check --json",
        "update --reinstall --json",
        "update --source /srv/feed --check",
        "update --startup-check false --check --json",
        "update --complete-install 1.2.3",
        "update --allow-unsigned",
        "update --allow-unsigned --reinstall --json",
    ] {
        assert!(parses(accepted), "failed to parse {accepted:?}");
    }
    for rejected in [
        "update --check --reinstall",
        "update --complete-install",
        "update --complete-install 1.2.3 --check",
        "update --complete-install 1.2.3 --source /srv/feed",
        "update --complete-install 1.2.3 --startup-check true",
        "update --startup-check maybe",
        "update --check --allow-unsigned",
        "update --complete-install 1.2.3 --allow-unsigned",
    ] {
        assert!(!parses(rejected), "unexpectedly parsed {rejected:?}");
    }
    let mut command = Cli::command();
    let help = command
        .find_subcommand_mut("update")
        .expect("update command")
        .render_long_help()
        .to_string();
    assert!(help.contains("--check") && help.contains("--allow-unsigned"));
    assert!(help.contains("--reinstall"));
    assert!(help.contains("~/.local/bin/se"));
    assert!(!help.contains("complete-install"));
}

#[test]
fn cli_task_update_completion_answer_and_source_rules() {
    let completed = CompletedInstall {
        version: "1.2.3".into(),
        worker: Some(WorkerHandoff::Replaced),
        worker_error: None,
    };
    let answer = serde_json::to_string(&completed).unwrap();
    assert_eq!(
        answer,
        r#"{"version":"1.2.3","worker":"replaced","worker_error":null}"#
    );
    let decoded: CompletedInstall = serde_json::from_str(&answer).unwrap();
    assert_eq!(decoded, completed);

    assert_eq!(
        validate_source("  https://example.test/feed  ").unwrap(),
        "https://example.test/feed"
    );
    assert!(validate_source("   ").is_err());
    assert!(validate_source("a\nb").is_err());
    assert!(validate_source(&"x".repeat(4097)).is_err());
}

#[test]
fn luecken_task_update_report_names_the_rollback_pause() {
    assert_eq!(paused_line(None), None);
    assert_eq!(
        paused_line(Some("0.5.170")).as_deref(),
        Some("auto_update\tpaused after a rollback to 0.5.170")
    );
}

/// Field failure 2026-11-17: `se update --check` showed the bare code only.
#[test]
fn update_sha_task_cli_names_reason_next_action_and_pending_update() {
    use crate::updater::{PendingUpdate, UpdateFailure};
    let failure = UpdateFailure {
        error: Some("runtime_sha256_mismatch".into()),
        reason: "Grund\tmit Tab".into(),
        next_action: "Weiter".into(),
        pending: Some(PendingUpdate::RollBack {
            previous_version: "0.5.197".into(),
            target_version: "0.5.198".into(),
        }),
    };
    assert_eq!(
        super::failure::lines(Some(&failure)),
        vec![
            "previous_update_error\truntime_sha256_mismatch",
            "previous_update_reason\tGrund mit Tab",
            "previous_update_next\tWeiter",
            "pending_update\troll_back\tv0.5.198 -> v0.5.197",
        ]
    );
    assert!(super::failure::lines(None).is_empty());
    let rollback = "Frühere Aktualisierung abschließen: update_rollback_failed: lock_busy: x";
    let explained = super::failure::explained(rollback.to_string());
    assert!(explained.starts_with(rollback), "{explained}");
    assert!(
        explained.contains("\nEine unterbrochene frühere Aktualisierung"),
        "{explained}"
    );
    let plain = "the feed offers 1.0, older than this se 2.0";
    assert_eq!(super::failure::explained(plain.to_string()), plain);
}
