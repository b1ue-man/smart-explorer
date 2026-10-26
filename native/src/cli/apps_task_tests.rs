//! App CLI data versions stay separate from the program version and completions.
use super::AppsCommand;
use crate::cli::{Cli, Command};
use clap::{error::ErrorKind, CommandFactory, Parser};

fn parsed_icon(extra: &[&str]) -> AppsCommand {
    let hash = "ab".repeat(32);
    let revision = "cd".repeat(32);
    let mut words: Vec<String> = [
        "se",
        "apps",
        "icon",
        "paired-device",
        "--app-id",
        "editor",
        "--sha256",
        &hash,
        "--width",
        "32",
        "--height",
        "48",
        "--catalog-revision",
        &revision,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    words.extend(extra.iter().map(|word| (*word).to_owned()));
    let cli = Cli::try_parse_from(words).unwrap_or_else(|error| panic!("{error}"));
    match cli.command {
        Command::Apps(args) => args.command,
        _ => panic!("the icon command must retain its Apps route"),
    }
}

#[test]
fn batches_app_cli_icon_data_version_retains_default_and_explicit_values() {
    for (extra, expected_version, expected_json) in [
        (&[][..], 1, false),
        (&["--icon-version", "2", "--json"][..], 2, true),
    ] {
        match parsed_icon(extra) {
            AppsCommand::Icon {
                device,
                app_id,
                sha256,
                width,
                height,
                icon_version,
                catalog_revision,
                json,
            } => {
                assert_eq!(device, "paired-device");
                assert_eq!(app_id, "editor");
                assert_eq!(sha256, "ab".repeat(32));
                assert_eq!((width, height), (32, 48));
                assert_eq!(icon_version, expected_version);
                assert_eq!(catalog_revision, "cd".repeat(32));
                assert_eq!(json, expected_json);
            }
            _ => panic!("the icon arguments must retain their complete body"),
        }
    }
}

#[test]
fn batches_app_cli_program_version_remains_available_at_every_apps_level() {
    for path in [
        &["se"][..],
        &["se", "apps"][..],
        &["se", "apps", "icon"][..],
    ] {
        for flag in ["-V", "--version"] {
            let words = path.iter().copied().chain([flag]);
            let error = match Cli::try_parse_from(words) {
                Err(error) => error,
                Ok(_) => panic!("{path:?} {flag} must display the program version"),
            };
            assert_eq!(error.kind(), ErrorKind::DisplayVersion);
            assert!(error.to_string().contains(env!("CARGO_PKG_VERSION")));
        }
    }
}

fn complete(words: &[&str]) -> Vec<String> {
    let args = words.iter().map(std::ffi::OsString::from).collect();
    clap_complete::engine::complete(&mut Cli::command(), args, words.len() - 1, None)
        .unwrap()
        .iter()
        .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn batches_app_cli_complete_tree_and_icon_options_remain_valid() {
    Cli::command().debug_assert();
    let commands = complete(&["se", "apps", ""]);
    for expected in [
        "catalog",
        "icon",
        "launch",
        "receipt",
        "rdp-probe",
        "local-grants",
        "set-local-grant",
    ] {
        assert!(commands.iter().any(|value| value == expected), "{commands:?}");
    }
    for (typed, expected) in [
        ("--app-", "--app-id"),
        ("--sha", "--sha256"),
        ("--wid", "--width"),
        ("--hei", "--height"),
        ("--icon-v", "--icon-version"),
        ("--catalog-r", "--catalog-revision"),
        ("--j", "--json"),
        ("--hel", "--help"),
        ("--ver", "--version"),
    ] {
        let options = complete(&["se", "apps", "icon", "paired-device", typed]);
        assert!(options.iter().any(|value| value == expected), "{options:?}");
    }
}
