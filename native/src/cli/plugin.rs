//! `se plugin`: installs the Smart Explorer plugin into AI agent hosts so they
//! start `se mcp` and know how to use it — Claude Code (plugin from a local
//! marketplace with server and skill) and Codex (MCP server entry and skill),
//! and prints the configuration for other MCP clients. docs/AI_AGENT_PLUGIN.md.
#[path = "plugin/claude.rs"]
mod claude;
#[path = "plugin/codex.rs"]
mod codex;
#[path = "plugin/config.rs"]
mod config;
#[path = "plugin/managed.rs"]
mod managed;
#[path = "plugin/files.rs"]
mod files;
#[path = "plugin/host_cli.rs"]
mod host_cli;

use clap::{Args, Subcommand, ValueEnum};
use serde_json::{json, Value};

const PLUGIN_HELP: &str = "\
Install the Smart Explorer plugin into AI coding agents. The agent then starts
`se mcp` itself and gets tools for files on saved remotes, Share peers, Google
Drive and local disks, remote commands over SSH or Share, and every other se
command, plus a skill that explains them. The plugin tells the agent to use
these tools before its built-in file tools and shell file commands.

Claude Code: se writes a local plugin marketplace into its app data and runs
`claude plugin marketplace add` and `claude plugin install
smart-explorer@smart-explorer`. Codex: a format-preserving MCP entry and a skill in
~/.agents/skills/smart-explorer. Both hosts use the verified installation
launcher when this installation has a managed runtime. Historical terminal-only
updates keep their absolute local se entry usable offline until a managed update
provides the launcher. New managed MCP processes select its confirmed runtime generation; an
existing compatible process retains its catalog. Use the host's documented
MCP restart action to load new capabilities.

Examples:
  se plugin install
  se plugin install claude
  se plugin status --json
  se plugin config generic
  se plugin uninstall codex";

#[derive(Args)]
#[command(long_about = PLUGIN_HELP)]
pub(super) struct PluginArgs {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Installer-only cleanup of the current installation's MCP program files.
    #[command(hide = true)]
    RetireInstallation,
    #[command(hide = true)]
    Refresh,
    #[command(about = "Install into Claude Code and/or Codex (default: every host found)")]
    Install(HostArgs),
    #[command(about = "Remove the plugin, server entry and generated files")]
    Uninstall(HostArgs),
    #[command(about = "Show which hosts are found and what is installed")]
    Status {
        #[arg(long, help = "Print machine-readable JSON")]
        json: bool,
    },
    #[command(about = "Print the MCP configuration for another client")]
    Config {
        #[arg(value_enum)]
        host: config::ConfigHost,
    },
}

#[derive(Args)]
struct HostArgs {
    #[arg(value_enum, default_value_t = Host::All)]
    host: Host,
    #[arg(long, help = "Print machine-readable JSON")]
    json: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Host {
    Claude,
    Codex,
    All,
}

impl Host {
    fn selected(self) -> &'static [Host] {
        match self {
            Host::Claude => &[Host::Claude],
            Host::Codex => &[Host::Codex],
            Host::All => &[Host::Claude, Host::Codex],
        }
    }
}

pub(super) fn run(args: PluginArgs) -> Result<i32, String> {
    if matches!(&args.command,Command::RetireInstallation) {
        crate::mcp_launcher::retire_current_installation()?;
        return Ok(0);
    }
    if matches!(&args.command, Command::Install(_) | Command::Uninstall(_) | Command::Refresh) && crate::mcp_launcher::current_runtime().is_some() {
        return Err("integration_maintenance_handoff_required: use the registered installation".into());
    }
    let _generator=if matches!(&args.command,Command::Install(_)|Command::Uninstall(_)|Command::Refresh) {
        crate::mcp_launcher::generator_operation()?
    }else{None};
    let _legacy_update=if matches!(&args.command,Command::Install(_)) {crate::mcp_launcher::legacy_profile_guard()?}else{None};
    let (executable, launcher_error) = match files::executable() {
        Ok(path)=>(path,None),
        Err(error) if matches!(&args.command,Command::Status {..}|Command::Uninstall(_)) => (std::env::current_exe().map_err(|e|e.to_string())?,Some(error)),
        Err(error)=>return Err(error),
    };
    match args.command {
        Command::RetireInstallation=>Ok(0),
        Command::Refresh => { files::refresh()?; Ok(0) },
        Command::Install(selection) => {
            let mut reports = Vec::new();
            let mut installed = 0usize;
            let mut failed = 0usize;
            for host in selection.host.selected() {
                let report = match host {
                    Host::Claude => claude::install(&executable),
                    _ => codex::install(&executable),
                };
                match report {
                    Ok(report) => {
                        installed += usize::from(report["installed"] == true);
                        reports.push(report);
                    }
                    Err(error) => {
                        failed += 1;
                        reports.push(
                            json!({ "host": label(*host), "installed": false, "error": error }),
                        );
                    }
                }
            }
            print(&reports, selection.json)?;
            // An explicitly named host must be installed; `all` needs one.
            let ok = failed == 0
                && (installed == selection.host.selected().len()
                    || (selection.host == Host::All && installed > 0));
            Ok(if ok { 0 } else { 1 })
        }
        Command::Uninstall(selection) => {
            let mut reports = Vec::new();
            for host in selection.host.selected() {
                let report = match host {
                    Host::Claude => claude::uninstall(),
                    _ => codex::uninstall(),
                };
                reports.push(
                    report.unwrap_or_else(|error| json!({ "host": label(*host), "error": error })),
                );
            }
            let failed = reports.iter().any(|report| report.get("error").is_some());
            print(&reports, selection.json)?;
            Ok(if failed { 1 } else { 0 })
        }
        Command::Status { json } => {
            let mut reports = vec![claude::status(&executable), codex::status(&executable)];
            reports.push(serde_json::json!({"host":"runtime","runtime":crate::mcp_launcher::status_value(),"launcher_error":launcher_error}));
            print(&reports, json)?;
            Ok(0)
        }
        Command::Config { host } => {
            print!("{}", config::render(host, &executable));
            Ok(0)
        }
    }
}

fn label(host: Host) -> &'static str {
    match host {
        Host::Claude => "claude",
        Host::Codex => "codex",
        Host::All => "all",
    }
}

fn print(reports: &[Value], json: bool) -> Result<(), String> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(reports).map_err(|error| error.to_string())?
        );
        return Ok(());
    }
    for report in reports {
        let host = report["host"].as_str().unwrap_or("?");
        if let Some(error) = report["error"].as_str() {
            println!("{host}\terror\t{error}");
        } else if report["installed"] == true {
            println!("{host}\tinstalled");
        } else if report.get("installed").is_some() {
            println!("{host}\tnot installed");
        } else {
            println!("{host}\t{}", report);
            continue;
        }
        match &report["next"] {
            Value::String(next) => println!("  next: {next}"),
            Value::Array(steps) => {
                for step in steps.iter().filter_map(Value::as_str) {
                    println!("  run: {step}");
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Called by `se mcp` at start: rewrites generated plugin files that an older
/// version or another path of `se` wrote.
pub(super) fn refresh_generated_files() {
    if crate::mcp_launcher::current_runtime().is_some() {
        if let Err(error) = crate::mcp_launcher::refresh_current_profile() { eprintln!("se plugin: {error}"); }
        return;
    }
    let refreshed=match crate::mcp_launcher::Installation::current() {
        Ok(None)=>files::refresh(),
        _=>crate::mcp_launcher::refresh_entry_installation(),
    };
    if let Err(error)=refreshed {eprintln!("se plugin: {error}");}
}

/// Generated-file state for the MCP `status` tool (no host commands run).
pub(super) fn status_value() -> Value {
    let executable = match files::executable() {Ok(path)=>path,Err(error)=>return json!({"runtime":crate::mcp_launcher::status_value(),"launcher_error":error})};
    json!({
        "claude": files::generated_state(Some(files::claude_marketplace_dir()), &executable),
        "codex_skill": files::generated_state(files::codex_skill_dir(), &executable),
    })
}
