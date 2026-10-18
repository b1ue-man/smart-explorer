mod background;
mod completions;
mod completions_access;
mod completions_requests;
mod completions_screen;
mod connections;
mod desktop;
mod doctor;
mod drive;
mod entry_kind;
mod exec;
mod exec_ssh;
mod host_key_prompt;
mod integration;
mod licenses;
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
mod screen;
mod search;
mod setup;
mod share;
mod shell;
mod shell_session;
mod sync_jobs;
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
  se sync list
  se sync edit Docs --trigger realtime
  se background status
  se desktop status --json
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
  se drive mount @prod:/srv --mount-point ~/SmartExplorer/prod
  se drive set <ID> --auto-remount on
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
    #[command(
        about = "Mount selected remotes as drives (Windows drive letters, Linux folders over FUSE)"
    )]
    Drive(drive::DriveArgs),
    #[command(about = "Manage saved remotes and Share contacts")]
    Connections(connections::ConnectionsArgs),
    #[command(about = "Manage saved sync setups (every setting of the GUI editor)")]
    Sync(sync_jobs::SyncArgs),
    #[command(about = "Background sync worker: login start, check interval, pause")]
    Background(background::BackgroundArgs),
    #[command(about = "Desktop integration settings (Windows Explorer context menu)")]
    Integration(integration::IntegrationArgs),
    #[command(
        about = "se Desktop: desktop surface with devices, rooms and vaults (Windows, Linux)"
    )]
    Desktop(desktop::DesktopArgs),
    #[command(about = "Screen of paired devices: devices, probe, measurement, sessions")]
    Screen(screen::ScreenArgs),
    #[command(about = "Generate live shell completion setup")]
    Completions(completions::CompletionsArgs),
    #[command(about = "Check the update feed and install a newer se (and app)")]
    Update(update::UpdateArgs),
    #[command(about = "Serve Smart Explorer to AI agents over MCP (stdio; started by the agent)")]
    Mcp(mcp::McpArgs),
    #[command(about = "Install the Smart Explorer plugin into AI agents (Claude Code, Codex)")]
    Plugin(plugin::PluginArgs),
    #[command(
        about = "Lizenztexte aller eingebetteten Rust-Crates ausgeben (--json: Übersicht mit SHA-256 und Crate-Liste)"
    )]
    Licenses(LicensesArgs),
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
        about = "Run a command on another device within the access it granted (commands/terminal: off, ask or allow)."
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

#[derive(Args)]
struct LicensesArgs {
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
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
        Command::Sync(args) => sync_jobs::run(args),
        Command::Background(args) => background::run(args),
        Command::Integration(args) => integration::run(args),
        Command::Desktop(args) => desktop::run(args),
        Command::Screen(args) => screen::run(args),
        Command::Completions(args) => completions::run(args),
        Command::Update(args) => update::run(args),
        Command::Mcp(args) => mcp::run(args),
        Command::Plugin(args) => plugin::run(args),
        Command::Licenses(args) => Ok(licenses::run(args.json)),
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
#[path = "parse_tests.rs"]
mod tests;
