//! Package G1 of plan 2026-10-31 §F: the rights options parse on every
//! command of the connect and share flow and fill exactly the rights the
//! preset and the single switches name.
use clap::Parser;

use super::*;
use crate::share::connection_rights::{ConnectionRights, FolderChoice, RightsPreset};

fn parses(command: &[&str]) -> bool {
    crate::cli::Cli::try_parse_from(command.iter().copied()).is_ok()
}

#[test]
fn abgleich_task_rights_options_parse_on_every_connect_command() {
    for command in [
        &["se", "share", "request", "accept", "--preset", "read-write"][..],
        &[
            "se", "share", "request", "accept", "dev", "--preset", "none",
        ],
        &[
            "se",
            "share",
            "request",
            "policy",
            "auto-accept",
            "--preset",
            "read-only",
        ],
        &[
            "se",
            "share",
            "discoverable",
            "--preset",
            "files-exec",
            "--minutes",
            "5",
        ],
        &["se", "share", "identity", "--preset", "all", "--json"],
        &["se", "share", "identity", "--clear-code-rights"],
        &[
            "se",
            "share",
            "room",
            "create",
            "--name",
            "Team",
            "--preset",
            "read-only",
        ],
        &["se", "share", "grants", "set", "dev", "--write"],
        &[
            "se",
            "share",
            "grants",
            "set",
            "dev",
            "--preset",
            "read-only",
            "--commands",
            "ask",
        ],
        &["se", "share", "grants", "presets", "--json"],
        &["se", "share", "grants", "pending"],
        &[
            "se", "share", "grants", "confirm", "dev", "--preset", "all", "--for", "1d",
        ],
        &[
            "se",
            "share",
            "grants",
            "confirm",
            "--room",
            "Team",
            "--preset",
            "read-only",
        ],
        &["se", "share", "grants", "cancel", "dev"],
        &["se", "share", "grants", "cancel", "--room", "Team"],
    ] {
        assert!(parses(command), "{command:?}");
    }
    for command in [
        &["se", "share", "request", "accept", "--preset", "everything"][..],
        &[
            "se",
            "share",
            "grants",
            "confirm",
            "dev",
            "--all-folders",
            "--no-folders",
        ],
        &[
            "se",
            "share",
            "grants",
            "confirm",
            "dev",
            "--write",
            "--read-only",
        ],
        &[
            "se",
            "share",
            "identity",
            "--clear-code-rights",
            "--preset",
            "all",
        ],
    ] {
        assert!(!parses(command), "{command:?}");
    }
}

#[test]
fn abgleich_task_preset_and_switches_give_exactly_these_rights() {
    let roots = vec![
        crate::share::SharedRoot::new("Docs", "/home/u/Docs"),
        crate::share::SharedRoot::new("Fotos", "/home/u/Fotos"),
    ];
    assert_eq!(RightsArgs::default().rights(None, &roots, 0).unwrap(), None);
    let args = RightsArgs {
        preset: Some(PresetArg::ReadWrite),
        folders: vec!["docs".into()],
        commands: Some(DecisionArg::Ask),
        duration: Some("1d".into()),
        ..RightsArgs::default()
    };
    let rights = args.rights(None, &roots, 1_000).unwrap().unwrap();
    assert_eq!(
        rights,
        ConnectionRights {
            folders: FolderChoice::Selected {
                paths: vec!["/home/u/Docs".into()],
                connections: false,
            },
            commands: crate::share::access_grant::CapabilityDecision::Ask,
            valid_until: Some(1_000 + 86_400),
            ..RightsPreset::ReadWrite.rights()
        }
    );
    // Without a preset the switches change the given base.
    let only_terminal = RightsArgs {
        terminal: Some(DecisionArg::Allow),
        ..RightsArgs::default()
    };
    let base = RightsPreset::ReadOnly.rights();
    let changed = only_terminal
        .rights(Some(base.clone()), &roots, 0)
        .unwrap()
        .unwrap();
    assert_eq!(changed.folders, base.folders);
    assert_eq!(
        changed.terminal,
        crate::share::access_grant::CapabilityDecision::Allow
    );
    let unknown = RightsArgs {
        folders: vec!["Musik".into()],
        ..RightsArgs::default()
    };
    assert!(unknown.rights(None, &roots, 0).is_err());
    assert!(parse_duration("7d").is_ok() && parse_duration("x").is_err());
    let value = rights_value(&RightsPreset::All.rights());
    assert_eq!(value["preset"], "all");
    assert_eq!(presets_value().as_array().map(Vec::len), Some(5));
}
