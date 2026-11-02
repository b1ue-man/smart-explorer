//! DSK-M3 signals of `se desktop`: every command and option parses, invalid
//! values are refused (exit code 2), completion, and the MCP wiring of R5
//! (`status` lists `desktop`, `INSTRUCTIONS` name it within the host limit).
use std::ffi::OsString;

use clap::{CommandFactory, Parser};

use super::{DesktopCommand, ForeignIcons, OpenWith, PartArg, Switch};
use crate::cli::Cli;
use crate::desktop::core::config::{
    Background, BackgroundKind, ForeignDesktopIcons, OpenDevicesWith,
};

#[derive(Parser)]
#[command(name = "se-desktop-task")]
struct Harness {
    #[command(subcommand)]
    command: DesktopCommand,
}

fn parse(args: &[&str]) -> Result<DesktopCommand, clap::Error> {
    let words = std::iter::once("se-desktop-task").chain(args.iter().copied());
    Harness::try_parse_from(words).map(|harness| harness.command)
}

#[test]
fn desktop_task_cli_parse() {
    assert_eq!(
        parse(&["status"]).unwrap(),
        DesktopCommand::Status { json: false }
    );
    assert_eq!(
        parse(&["status", "--json"]).unwrap(),
        DesktopCommand::Status { json: true }
    );
    assert_eq!(
        parse(&["enable"]).unwrap(),
        DesktopCommand::Enable {
            parts: Vec::new(),
            json: false
        }
    );
    assert_eq!(
        parse(&[
            "enable",
            "--part",
            "surface",
            "--part",
            "nav-pane",
            "--part",
            "jump-list",
            "--json"
        ])
        .unwrap(),
        DesktopCommand::Enable {
            parts: vec![PartArg::Surface, PartArg::NavPane, PartArg::JumpList],
            json: true
        }
    );
    assert_eq!(
        parse(&["disable"]).unwrap(),
        DesktopCommand::Disable { json: false }
    );
    assert_eq!(
        parse(&["remove", "--json"]).unwrap(),
        DesktopCommand::Remove { json: true }
    );
    assert_eq!(
        parse(&["part", "jump-list", "off"]).unwrap(),
        DesktopCommand::Part {
            part: PartArg::JumpList,
            state: Switch::Off,
            json: false
        }
    );
    assert_eq!(
        parse(&["part", "surface", "on", "--json"]).unwrap(),
        DesktopCommand::Part {
            part: PartArg::Surface,
            state: Switch::On,
            json: true
        }
    );
    assert_eq!(
        parse(&["restart", "--json"]).unwrap(),
        DesktopCommand::Restart { json: true }
    );

    let DesktopCommand::Config(args) = parse(&[
        "config",
        "--background",
        "color:#11AA33",
        "--desktop-files",
        "off",
        "--devices",
        "on",
        "--rooms",
        "off",
        "--vaults",
        "on",
        "--screens",
        "off",
        "--recycle-bin",
        "on",
        "--this-pc",
        "off",
        "--open-devices-with",
        "file-manager",
        "--foreign-desktop-icons",
        "take-over",
        "--json",
    ])
    .unwrap() else {
        panic!("config expected");
    };
    assert_eq!(args.open_devices_with, Some(OpenWith::FileManager));
    assert_eq!(args.foreign_desktop_icons, Some(ForeignIcons::TakeOver));
    let change = args.change();
    assert_eq!(
        change.background,
        Some(Background {
            kind: BackgroundKind::Color,
            value: Some("#11AA33".into())
        })
    );
    assert_eq!(
        (
            change.desktop_files,
            change.devices,
            change.rooms,
            change.vaults,
            change.screens
        ),
        (
            Some(false),
            Some(true),
            Some(false),
            Some(true),
            Some(false)
        )
    );
    assert_eq!(
        (change.recycle_bin, change.this_pc),
        (Some(true), Some(false))
    );
    assert_eq!(change.open_devices_with, Some(OpenDevicesWith::FileManager));
    assert_eq!(
        change.foreign_desktop_icons,
        Some(ForeignDesktopIcons::TakeOver)
    );
    assert!(args.json);
    for background in [
        "system",
        "transparent",
        "image:/home/nutzer/Bilder/berg.png",
    ] {
        assert!(
            parse(&["config", "--background", background]).is_ok(),
            "{background}"
        );
    }
    let DesktopCommand::Config(empty) = parse(&["config"]).unwrap() else {
        panic!("config expected");
    };
    assert_eq!(empty.change(), Default::default());

    let invalid: &[&[&str]] = &[
        &[],
        &["start"],
        &["enable", "--part", "taskbar"],
        &["enable", "--part"],
        &["part", "surface"],
        &["part", "surface", "maybe"],
        &["part", "desktop", "on"],
        &["config", "--background", "blue"],
        &["config", "--background", "color:#12345"],
        &["config", "--background", "image:"],
        &["config", "--devices", "yes"],
        &["config", "--open-devices-with", "explorer"],
        &["config", "--foreign-desktop-icons", "drop"],
        &["status", "--part", "surface"],
        &["remove", "--force"],
        &["restart", "now"],
    ];
    for args in invalid {
        let error = parse(args)
            .err()
            .unwrap_or_else(|| panic!("{args:?} accepted"));
        assert_eq!(error.exit_code(), 2, "{args:?}");
    }
    // R5 registers the command under `se`.
    assert!(Cli::try_parse_from(["se", "desktop", "status", "--json"]).is_ok());
}

fn complete(words: &[&str], index: usize) -> Vec<String> {
    let words: Vec<OsString> = words.iter().map(OsString::from).collect();
    clap_complete::engine::complete(&mut Cli::command(), words, index, None)
        .unwrap()
        .iter()
        .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn desktop_task_cli_completion_lists_desktop() {
    assert!(complete(&["se", "des"], 1).contains(&"desktop".to_string()));
    let commands = complete(&["se", "desktop", ""], 2);
    for name in [
        "status", "enable", "disable", "remove", "part", "config", "restart",
    ] {
        assert!(commands.contains(&name.to_string()), "{name}: {commands:?}");
    }
    let lists: [(&[&str], usize, &[&str]); 5] = [
        (
            &["se", "desktop", "part", ""],
            3,
            &["surface", "nav-pane", "jump-list"],
        ),
        (&["se", "desktop", "part", "surface", ""], 4, &["on", "off"]),
        (
            &["se", "desktop", "enable", "--part", ""],
            4,
            &["surface", "nav-pane", "jump-list"],
        ),
        (
            &["se", "desktop", "config", "--open-devices-with", ""],
            4,
            &["app", "file-manager"],
        ),
        (
            &["se", "desktop", "config", "--foreign-desktop-icons", ""],
            4,
            &["keep", "take-over"],
        ),
    ];
    for (words, index, expected) in lists {
        let found = complete(words, index);
        for value in expected {
            assert!(found.contains(&value.to_string()), "{words:?}: {found:?}");
        }
    }
}

/// The string literal that starts right after `marker` in Rust source.
fn rust_literal_after(source: &str, marker: &str) -> String {
    let start = source.find(marker).expect("marker") + marker.len();
    let mut chars = source[start..].chars().peekable();
    assert_eq!(chars.next(), Some('"'));
    let mut text = String::new();
    while let Some(character) = chars.next() {
        match character {
            '"' => return text,
            '\\' => match chars.next() {
                Some('\n') | Some('\r') => {
                    while chars.peek().is_some_and(|next| next.is_whitespace()) {
                        chars.next();
                    }
                }
                Some('n') => text.push('\n'),
                Some('t') => text.push('\t'),
                Some(other) => text.push(other),
                None => break,
            },
            '\r' => {}
            other => text.push(other),
        }
    }
    panic!("unterminated literal");
}

#[test]
fn desktop_task_mcp_status_has_desktop() {
    // R5 adds the object to MCP `status` (DSK-M3, C7 O1).
    let source = include_str!("../mcp/status_tools.rs");
    assert!(
        source.contains("\"desktop\": crate::desktop::status_value()"),
        "MCP status lacks se Desktop"
    );
    let value = crate::desktop::status_value();
    for key in [
        "platform",
        "state",
        "parts",
        "processes",
        "residue",
        "autostart",
    ] {
        assert!(value.get(key).is_some(), "{key}");
    }
}

#[test]
fn desktop_task_mcp_instructions_within_limit() {
    let source = include_str!("../mcp/catalog.rs");
    let instructions = rust_literal_after(source, "pub(super) const INSTRUCTIONS: &str = ");
    assert!(instructions.len() <= 2_048, "{}", instructions.len());
    let first = instructions.split("\n\n").next().unwrap_or_default();
    assert!(first.len() <= 512, "{}", first.len());
    let desktop = instructions
        .find("se Desktop (`desktop`)")
        .expect("INSTRUCTIONS name se Desktop (V12)");
    // V12 order of the third addition: drives → vaults → se Desktop.
    for earlier in ["drives (`drive`)", "vaults (`vault`)"] {
        if let Some(at) = instructions.find(earlier) {
            assert!(at < desktop, "{earlier} after se Desktop");
        }
    }
    assert!(source.contains(r#"[\"desktop\",\"status\",\"--json\"]"#));
    assert!(source.contains(r#"[\"desktop\",\"enable\",\"--json\"]"#));
}
