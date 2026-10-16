mod completions;
mod completions_requests;
mod connections;
mod doctor;
mod drive;
mod entry_kind;
mod exec;
mod exec_ssh;
mod host_key_prompt;
mod link_paths;
#[cfg(test)]
mod links_task_backends;
#[cfg(test)]
mod links_task_tests;
mod mcp;
mod ops;
#[cfg(test)]
mod ops_safety_tests;
#[cfg(test)]
mod ops_search_tests;
#[cfg(test)]
mod ops_transfer_tests;
mod os;
mod plugin;
mod private_file;
mod search;
mod setup;
mod share;
mod shell;
mod shell_session;
mod target;
mod transfer;
mod tree_apply;
mod tree_destination;
mod tree_guard;
mod tree_observer;
mod tree_ops;
mod tree_plan;
#[cfg(test)]
mod tree_preflight_tests;
mod tree_remove;
mod tree_spool;
mod update;

use clap::{Args, CommandFactory, Parser, Subcommand};
use std::io::{self, Write};

const CLI_HELP: &str = "\
Smart Explorer terminal access to the same saved remotes, Share peers,
credential store, and background worker used by the GUI.

Targets:
  @label:/path               saved connection shorthand
  sftp://user@host:22/path   saved endpoint (exact user/host/port match)
  webdav://user@host:443/path saved endpoint (exact user/host/port match)
  share://direct/id/path     Smart Explorer peer endpoint
  C:\\local\\path or ./path   local filesystem path

Examples:
  se connections list
  se connections add-peer --code SE-D3-... --name Laptop
  se ls @prod:/srv
  se get @prod:/srv/report.txt .\\report.txt
  se cp -r @prod:/exports share://direct/peer-id/Drop
  se doctor --json
  se share status --json
  se share discoverable --minutes 5 --pin-stdin
  se update --check
  se shell @prod:/srv --attachable
  se plugin install
  se drive runtime
  se drive install-runtime
  se drive mount @prod:/srv --letter M
  source <(se completions bash)";

#[derive(Parser)]
#[command(
    name = "se",
    version,
    about = "Smart Explorer terminal remote operations",
    long_about = CLI_HELP
)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Diagnose terminal configuration without starting the GUI")]
    Doctor(doctor::DoctorArgs),
    #[command(about = "Configure and manage headless Smart Explorer Share")]
    Share(share::ShareArgs),
    #[command(about = "Mount selected remotes as Windows drive letters")]
    Drive(drive::DriveArgs),
    #[command(about = "Manage saved remotes and Share contacts")]
    Connections(connections::ConnectionsArgs),
    #[command(about = "Generate live shell completion setup")]
    Completions(completions::CompletionsArgs),
    #[command(about = "Check the update feed and install a newer se (and app)")]
    Update(update::UpdateArgs),
    #[command(about = "Serve Smart Explorer to AI agents over MCP (stdio; started by the agent)")]
    Mcp(mcp::McpArgs),
    #[command(about = "Install the Smart Explorer plugin into AI agents (Claude Code, Codex)")]
    Plugin(plugin::PluginArgs),
    #[command(about = "List a directory")]
    Ls(PathArg),
    #[command(about = "Show file or directory metadata")]
    Stat(PathArg),
    #[command(about = "Print a remote or local file")]
    Cat(PathArg),
    #[command(about = "Download from remote to local")]
    Get(CopyArgs),
    #[command(about = "Upload from local to remote")]
    Put(CopyArgs),
    #[command(
        about = "Copy between local, saved, endpoint, or Share targets",
        long_about = "Copy between local, saved, endpoint, or Share targets. A link named as source or destination is followed (directory links only on local paths); links and special files inside a copied directory are skipped and listed on stderr."
    )]
    Cp(CopyArgs),
    #[command(
        about = "Move or rename between targets",
        long_about = "Move or rename between targets. Within one connection the entry is renamed (a link moves as itself); otherwise it is copied and the source removed. Links and special files inside a moved directory stay in the source, and so do the directories holding them; both are listed on stderr."
    )]
    Mv(MoveArgs),
    #[command(about = "Remove a file or directory")]
    Rm(RemoveArgs),
    #[command(about = "Create a directory")]
    Mkdir(PathArg),
    #[command(
        about = "Search names below a target",
        long_about = "Search names below a target. Prints `kind<TAB>size<TAB>relative path` per hit; kind is - (file), d (directory), l (link) or s (FIFO, socket, device). Links are found by name but never entered."
    )]
    Search(SearchArgs),
    #[command(
        about = "Run an unrestricted command on an explicitly authorized Share peer or saved SSH remote"
    )]
    Exec(exec::ExecArgs),
    #[command(
        about = "Open a persistent shell on a Share peer or saved SSH remote, or join a running session"
    )]
    Shell(shell::ShellArgs),
}

#[derive(Args)]
struct PathArg {
    #[arg(help = "Target path, endpoint, or @label:/path shorthand")]
    target: String,
}

#[derive(Args)]
struct CopyArgs {
    #[arg(help = "Source target")]
    src: String,
    #[arg(help = "Destination target")]
    dst: String,
    #[arg(short, long, help = "Allow directory copy")]
    recursive: bool,
    #[arg(short, long, help = "Allow overwriting an existing destination")]
    force: bool,
}

#[derive(Args)]
struct MoveArgs {
    #[arg(help = "Source target")]
    src: String,
    #[arg(help = "Destination target")]
    dst: String,
    #[arg(short, long, help = "Allow moving directories recursively")]
    recursive: bool,
    #[arg(short, long, help = "Allow overwriting an existing destination")]
    force: bool,
}

#[derive(Args)]
struct RemoveArgs {
    #[arg(help = "Target to remove")]
    target: String,
    #[arg(short, long, help = "Allow directory delete")]
    recursive: bool,
    #[arg(short, long, help = "Required for destructive delete")]
    force: bool,
    #[arg(
        long,
        help = "Allow deleting a filesystem root or saved connection's configured root"
    )]
    no_preserve_root: bool,
}

#[derive(Args)]
struct SearchArgs {
    #[arg(help = "Directory target to search below")]
    target: String,
    #[arg(help = "Search text or glob pattern")]
    query: String,
    #[arg(long, help = "Treat query as a glob pattern")]
    glob: bool,
    #[arg(
        long,
        default_value_t = 0,
        help = "Maximum result count; 0 means unlimited"
    )]
    max_results: u64,
    #[arg(
        long,
        help = "Also return matching directories (files, links and special files always)"
    )]
    dirs: bool,
}

pub fn run() -> i32 {
    // Completion registration and callbacks must run before argument parsing or
    // any other stdout output. The generated shell integration invokes this
    // same binary with COMPLETE=<shell> for live selector candidates.
    clap_complete::CompleteEnv::with_factory(Cli::command)
        .bin("se")
        .complete();
    match run_inner(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            let _ = writeln!(io::stderr(), "se: {e}");
            1
        }
    }
}

fn run_inner(cli: Cli) -> Result<i32, String> {
    match cli.command {
        Command::Doctor(args) => doctor::run(args),
        Command::Share(args) => share::run(args),
        Command::Drive(args) => drive::run(args),
        Command::Connections(args) => connections::run(args),
        Command::Completions(args) => completions::run(args),
        Command::Update(args) => update::run(args),
        Command::Mcp(args) => mcp::run(args),
        Command::Plugin(args) => plugin::run(args),
        Command::Ls(args) => {
            let t = target::resolve(&args.target)?;
            ops::list(&t)?;
            Ok(0)
        }
        Command::Stat(args) => {
            let t = target::resolve(&args.target)?;
            ops::stat(&t)?;
            Ok(0)
        }
        Command::Cat(args) => {
            let t = target::resolve(&args.target)?;
            ops::cat(&t)?;
            Ok(0)
        }
        Command::Get(args) | Command::Put(args) | Command::Cp(args) => {
            let src = target::resolve(&args.src)?;
            let dst = target::resolve(&args.dst)?;
            ops::copy(&src, &dst, args.recursive, args.force)?;
            Ok(0)
        }
        Command::Mv(args) => {
            let src = target::resolve(&args.src)?;
            let dst = target::resolve(&args.dst)?;
            ops::move_path(&src, &dst, args.recursive, args.force)?;
            Ok(0)
        }
        Command::Rm(args) => {
            let t = target::resolve(&args.target)?;
            ops::remove(&t, args.recursive, args.force, args.no_preserve_root)?;
            Ok(0)
        }
        Command::Mkdir(args) => {
            let t = target::resolve(&args.target)?;
            t.backend.mkdir_all(&t.path).map_err(|e| e.to_string())?;
            Ok(0)
        }
        Command::Search(args) => {
            let t = target::resolve(&args.target)?;
            search::search(&t, &args.query, args.glob, args.max_results, args.dirs)?;
            Ok(0)
        }
        Command::Exec(args) => exec::run(args),
        Command::Shell(args) => shell::run(args),
    }
}

#[cfg(test)]
mod tests {
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
}
