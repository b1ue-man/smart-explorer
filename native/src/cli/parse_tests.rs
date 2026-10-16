//! Parser and help tests of the top-level `se` commands.
use clap::{CommandFactory, Parser};

use super::Cli;

#[test]
fn parses_exec_argv_after_separator() {
    let cli = Cli::parse_from(["se", "exec", "share://direct/c", "--", "echo", "-n", "hi"]);
    match cli.command {
        super::Command::Exec(args) => {
            assert_eq!(args.target.as_deref(), Some("share://direct/c"));
            assert_eq!(args.argv, ["echo", "-n", "hi"]);
        }
        _ => panic!("wrong command"),
    }
}

#[test]
fn parses_recursive_force_copy_and_remove() {
    let cli = Cli::parse_from(["se", "cp", "-r", "-f", "@a:/dir", "@b:/dir"]);
    match cli.command {
        super::Command::Cp(args) => {
            assert!(args.recursive);
            assert!(args.force);
            assert_eq!(args.src, "@a:/dir");
            assert_eq!(args.dst, "@b:/dir");
        }
        _ => panic!("wrong command"),
    }

    let cli = Cli::parse_from([
        "se",
        "rm",
        "--recursive",
        "--force",
        "--no-preserve-root",
        "@a:/dir",
    ]);
    match cli.command {
        super::Command::Rm(args) => {
            assert!(args.recursive);
            assert!(args.force);
            assert!(args.no_preserve_root);
            assert_eq!(args.target, "@a:/dir");
        }
        _ => panic!("wrong command"),
    }
}

#[test]
fn parses_shell_exec_without_argv() {
    let cli = Cli::parse_from(["se", "exec", "share://direct/c", "--shell", "echo hi"]);
    match cli.command {
        super::Command::Exec(args) => {
            assert_eq!(args.shell.as_deref(), Some("echo hi"));
            assert!(args.argv.is_empty());
        }
        _ => panic!("wrong command"),
    }
}

#[test]
fn exec_accepts_context_free_form_for_runtime_selection() {
    let cli = Cli::parse_from(["se", "exec", "--", "echo", "hello"]);
    match cli.command {
        super::Command::Exec(args) => assert!(args.target.is_none()),
        _ => panic!("wrong command"),
    }
}

#[test]
fn parses_doctor_and_headless_share_commands() {
    assert!(Cli::try_parse_from(["se", "doctor", "--json"]).is_ok());
    assert!(Cli::try_parse_from(["se", "share"]).is_ok());
    assert!(Cli::try_parse_from([
        "se",
        "share",
        "configure",
        "--server",
        "wss://share.example.test"
    ])
    .is_ok());
    assert!(Cli::try_parse_from([
        "se",
        "share",
        "request",
        "accept",
        "device-a",
        "--fingerprint",
        "0011"
    ])
    .is_ok());
}

#[test]
fn top_level_help_shows_targets_and_setup_examples() {
    let help = Cli::command().render_long_help().to_string();
    assert!(help.contains("@label:/path"));
    assert!(help.contains("se connections add-peer"));
    assert!(help.contains("share://direct/id/path"));
}

#[test]
fn help_exposes_exec_as_available_and_explains_context_free_discovery() {
    let top = Cli::command().render_long_help().to_string();
    assert!(top.contains("Run an unrestricted command"));
    assert!(!top
        .to_ascii_lowercase()
        .contains("remote execution is disabled"));

    let exec = nested_help(&["exec"]);
    assert!(exec.contains("unrestricted shell authority"));
    assert!(exec.contains("target may be omitted"));
    assert!(exec.contains("--shell"));
    assert!(exec.contains("--timeout"));

    let requests = nested_help(&["share", "request"]);
    for command in ["list", "show", "accept", "reject", "delete"] {
        assert!(
            requests.contains(command),
            "missing {command} in {requests}"
        );
    }

    let grants = nested_help(&["share", "grants", "exec"]);
    assert!(grants.contains("enable"));
    assert!(grants.contains("disable"));

    let jobs = nested_help(&["share", "exec"]);
    for command in ["list", "show", "cancel", "history"] {
        assert!(jobs.contains(command), "missing {command} in {jobs}");
    }
}

#[test]
fn shell_session_task_cli_parses_shell_sessions_and_attach() {
    let cli = Cli::parse_from([
        "se",
        "shell",
        "@prod:/srv",
        "--attachable",
        "--name",
        "build",
        "--env",
        "A=1",
    ]);
    match cli.command {
        super::Command::Shell(args) => {
            assert_eq!(args.target.as_deref(), Some("@prod:/srv"));
            assert!(args.attachable);
            assert_eq!(args.name.as_deref(), Some("build"));
            assert_eq!(args.env, ["A=1"]);
        }
        _ => panic!("wrong command"),
    }
    assert!(Cli::try_parse_from(["se", "shell", "--attach", "build"]).is_ok());
    assert!(Cli::try_parse_from(["se", "shell", "@prod", "--attach", "build"]).is_err());
    assert!(Cli::try_parse_from(["se", "shell", "--list", "--json"]).is_ok());
    assert!(Cli::try_parse_from(["se", "shell", "--json"]).is_err());
    let help = nested_help(&["shell"]);
    for word in [
        "--attachable",
        "--attach",
        "--list",
        "--no-tty",
        "Ctrl+C",
        "~.",
    ] {
        assert!(help.contains(word), "missing {word} in {help}");
    }
}

#[test]
fn terminal_task_cli_parses_the_terminal_options_of_shell() {
    let parse = |arguments: &[&str]| {
        let mut all = vec!["se", "shell"];
        all.extend_from_slice(arguments);
        Cli::try_parse_from(all).map(|cli| match cli.command {
            super::Command::Shell(args) => args,
            _ => panic!("wrong command"),
        })
    };
    let plain = parse(&["@prod"]).unwrap();
    assert!(!plain.no_tty);
    assert_eq!(plain.escape_char.0, Some(b'~'));
    assert!(parse(&["@prod", "-T"]).unwrap().no_tty);
    assert!(parse(&["@prod", "--no-tty"]).unwrap().no_tty);
    // Joining a command session with -T is allowed.
    let attached = parse(&["--attach", "build", "-T"]).unwrap();
    assert!(attached.no_tty && attached.attach.as_deref() == Some("build"));
    let escape = |value: &str| parse(&["@prod", "-e", value]).map(|args| args.escape_char.0);
    assert_eq!(escape("none").unwrap(), None);
    assert_eq!(escape("#").unwrap(), Some(b'#'));
    for bad in ["ab", ".", ""] {
        assert!(escape(bad).is_err(), "{bad:?} must be refused");
    }
    assert!(parse(&["@prod", "--escape-char", "none"]).is_ok());
    assert!(parse(&["--attach", "build", "@prod"]).is_err());
}

fn nested_help(path: &[&str]) -> String {
    let mut command = Cli::command();
    for name in path {
        command = command
            .find_subcommand(name)
            .unwrap_or_else(|| panic!("missing command {name}"))
            .clone();
    }
    command.render_long_help().to_string()
}
