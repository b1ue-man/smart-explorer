mod completions;
mod connections;
mod doctor;
mod ops;
#[cfg(test)]
mod ops_safety_tests;
#[cfg(test)]
mod ops_search_tests;
#[cfg(test)]
mod ops_transfer_tests;
mod os;
mod setup;
mod share;
mod target;
mod transfer;
mod tree_apply;
mod tree_destination;
mod tree_guard;
mod tree_ops;
mod tree_plan;
#[cfg(test)]
mod tree_preflight_tests;
mod tree_remove;
mod tree_spool;

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
    #[command(about = "Manage saved remotes and Share contacts")]
    Connections(connections::ConnectionsArgs),
    #[command(about = "Generate live shell completion setup")]
    Completions(completions::CompletionsArgs),
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
    #[command(about = "Copy between local, saved, endpoint, or Share targets")]
    Cp(CopyArgs),
    #[command(about = "Move or rename between targets")]
    Mv(MoveArgs),
    #[command(about = "Remove a file or directory")]
    Rm(RemoveArgs),
    #[command(about = "Create a directory")]
    Mkdir(PathArg),
    #[command(about = "Search names below a target")]
    Search(SearchArgs),
    #[command(about = "Unavailable: remote execution is disabled in this release")]
    Exec(ExecArgs),
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
    #[arg(long, help = "Return matching directories instead of files")]
    dirs: bool,
}

#[derive(Args)]
struct ExecArgs {
    #[arg(help = "Share peer endpoint, such as share://direct/id")]
    target: String,
    #[arg(long, help = "Remote working directory")]
    cwd: Option<String>,
    #[arg(long, default_value_t = 30, help = "Remote timeout in seconds")]
    timeout: u64,
    #[arg(
        long,
        default_value_t = 1024 * 1024,
        help = "Maximum combined stdout and stderr bytes returned"
    )]
    max_output: u64,
    #[arg(
        long,
        help = "Keep the remote exit status even when returned output was truncated"
    )]
    allow_truncated_output: bool,
    #[arg(long, help = "Run one shell command under the full-code permission")]
    shell: Option<String>,
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        help = "Program and arguments after --"
    )]
    argv: Vec<String>,
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
        Command::Connections(args) => connections::run(args),
        Command::Completions(args) => completions::run(args),
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
            ops::search(&t, &args.query, args.glob, args.max_results, args.dirs)?;
            Ok(0)
        }
        Command::Exec(args) => exec(args),
    }
}

fn exec(args: ExecArgs) -> Result<i32, String> {
    let ExecArgs {
        target: _,
        cwd: _,
        timeout: _,
        max_output: _,
        allow_truncated_output: _,
        shell: _,
        argv: _,
    } = args;
    Err("remote execution is unsupported in this release".to_string())
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
                assert_eq!(args.target, "share://direct/c");
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
        let cli = Cli::parse_from([
            "se",
            "exec",
            "share://direct/c",
            "--allow-truncated-output",
            "--shell",
            "echo hi",
        ]);
        match cli.command {
            super::Command::Exec(args) => {
                assert_eq!(args.shell.as_deref(), Some("echo hi"));
                assert!(args.argv.is_empty());
                assert!(args.allow_truncated_output);
            }
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn exec_fails_before_contacting_the_worker() {
        let cli = Cli::parse_from(["se", "exec", "share://direct/c", "--", "echo", "hello"]);
        assert_eq!(
            super::run_inner(cli).unwrap_err(),
            "remote execution is unsupported in this release"
        );
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
}
