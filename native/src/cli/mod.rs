mod connections;
mod ops;
mod setup;
mod target;

use clap::{Args, Parser, Subcommand};
use std::io::{self, Write};

#[derive(Parser)]
#[command(
    name = "se",
    version,
    about = "Smart Explorer terminal remote operations"
)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Connections(connections::ConnectionsArgs),
    Ls(PathArg),
    Stat(PathArg),
    Cat(PathArg),
    Get(CopyArgs),
    Put(CopyArgs),
    Cp(CopyArgs),
    Mv(MoveArgs),
    Rm(RemoveArgs),
    Mkdir(PathArg),
    Search(SearchArgs),
    Exec(ExecArgs),
}

#[derive(Args)]
struct PathArg {
    target: String,
}

#[derive(Args)]
struct CopyArgs {
    src: String,
    dst: String,
    #[arg(short, long)]
    recursive: bool,
    #[arg(short, long)]
    force: bool,
}

#[derive(Args)]
struct MoveArgs {
    src: String,
    dst: String,
    #[arg(short, long)]
    recursive: bool,
    #[arg(short, long)]
    force: bool,
}

#[derive(Args)]
struct RemoveArgs {
    target: String,
    #[arg(short, long)]
    recursive: bool,
    #[arg(short, long)]
    force: bool,
}

#[derive(Args)]
struct SearchArgs {
    target: String,
    query: String,
    #[arg(long)]
    glob: bool,
    #[arg(long, default_value_t = 0)]
    max_results: u64,
    #[arg(long)]
    dirs: bool,
}

#[derive(Args)]
struct ExecArgs {
    target: String,
    #[arg(long)]
    cwd: Option<String>,
    #[arg(long, default_value_t = 30)]
    timeout: u64,
    #[arg(long, default_value_t = 1024 * 1024)]
    max_output: u64,
    #[arg(long)]
    shell: Option<String>,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
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
            ops::remove(&t, args.recursive, args.force)?;
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
    if result.timed_out {
        return Ok(124);
    }
    Ok(result.exit_code.unwrap_or(1).clamp(0, 255))
}

#[cfg(test)]
mod tests {
    use clap::Parser;

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

        let cli = Cli::parse_from(["se", "rm", "--recursive", "--force", "@a:/dir"]);
        match cli.command {
            super::Command::Rm(args) => {
                assert!(args.recursive);
                assert!(args.force);
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
}
