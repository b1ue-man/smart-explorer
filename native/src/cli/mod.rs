mod connections;
mod ops;
#[cfg(test)]
mod ops_safety_tests;
#[cfg(test)]
mod ops_search_tests;
#[cfg(test)]
mod ops_transfer_tests;
mod os;
mod setup;
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

use clap::{Args, Parser, Subcommand};
use std::io::{self, Write};

const CLI_HELP: &str = "\
Smart Explorer terminal access to the same saved remotes, Share peers, keyring
secrets, and background worker used by the GUI.

Targets:
  @label:/path               saved connection shorthand
  sftp://host/path           full endpoint
  webdav://host/path         full endpoint
  share://direct/id/path     Smart Explorer peer endpoint
  C:\\local\\path or ./path   local filesystem path

Examples:
  se connections list
  se connections add-peer --code SE-D3-... --name Laptop
  se ls @prod:/srv
  se get @prod:/srv/report.txt .\\report.txt
  se cp -r @prod:/exports share://direct/peer-id/Drop
  se exec share://direct/peer-id -- powershell -NoProfile -Command hostname";

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
    #[command(about = "Manage saved remotes and Share contacts")]
    Connections(connections::ConnectionsArgs),
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
    #[command(about = "Run a command on an allowed Smart Explorer Share peer")]
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
        Command::Connections(args) => connections::run(args),
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
    let (target, _) = crate::share::PeerOpenTarget::from_endpoint(&args.target)
        .ok_or_else(|| "exec target must be a share:// endpoint".to_string())?;
    let (argv, shell) = match (args.shell, args.argv.is_empty()) {
        (Some(cmd), true) => (vec![cmd], true),
        (Some(_), false) => return Err("use either --shell or argv after --, not both".into()),
        (None, true) => return Err("missing command; pass argv after -- or use --shell".into()),
        (None, false) => (args.argv, false),
    };
    let req = crate::share::ExecRequest {
        argv,
        cwd: args.cwd,
        timeout_ms: args.timeout.saturating_mul(1000),
        max_output_bytes: args.max_output,
        shell,
    };
    let result = crate::daemon::exec_share(target, req)?;
    io::stdout()
        .write_all(&result.stdout)
        .map_err(|e| e.to_string())?;
    io::stderr()
        .write_all(&result.stderr)
        .map_err(|e| e.to_string())?;
    if result.stdout_truncated || result.stderr_truncated {
        let streams = match (result.stdout_truncated, result.stderr_truncated) {
            (true, true) => "stdout and stderr",
            (true, false) => "stdout",
            (false, true) => "stderr",
            (false, false) => unreachable!(),
        };
        writeln!(
            io::stderr(),
            "\nse: warning: remote {streams} was truncated at the enforced output limit"
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(exec_result_code(&result, args.allow_truncated_output))
}

fn exec_result_code(result: &crate::share::ExecResult, allow_truncated_output: bool) -> i32 {
    if (result.stdout_truncated || result.stderr_truncated) && !allow_truncated_output {
        return 125;
    }
    if result.timed_out {
        return 124;
    }
    result.exit_code.unwrap_or(1).clamp(0, 255)
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
    fn truncated_exec_output_has_a_distinct_failure_status() {
        let result = crate::share::ExecResult {
            stdout: Vec::new(),
            stderr: Vec::new(),
            exit_code: Some(0),
            timed_out: false,
            stdout_truncated: true,
            stderr_truncated: false,
        };
        assert_eq!(super::exec_result_code(&result, false), 125);
        assert_eq!(super::exec_result_code(&result, true), 0);
    }

    #[test]
    fn top_level_help_shows_targets_and_setup_examples() {
        let help = Cli::command().render_long_help().to_string();
        assert!(help.contains("@label:/path"));
        assert!(help.contains("se connections add-peer"));
        assert!(help.contains("share://direct/id/path"));
    }
}
