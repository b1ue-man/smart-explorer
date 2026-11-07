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
