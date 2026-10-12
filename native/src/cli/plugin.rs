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
command, plus a skill that explains them.

Claude Code: se writes a local plugin marketplace into its app data and runs
`claude plugin marketplace add` and `claude plugin install
smart-explorer@smart-explorer`. Codex: `codex mcp add smart-explorer -- <se> mcp`
and a skill in ~/.agents/skills/smart-explorer. Both use the absolute path of
this se; `se mcp` refreshes the generated files after an update. Start a new
agent session afterwards.

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
    let executable = files::executable()?;
    match args.command {
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
            let reports = vec![claude::status(&executable), codex::status(&executable)];
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
    files::refresh();
}

/// Generated-file state for the MCP `status` tool (no host commands run).
pub(super) fn status_value() -> Value {
    let Ok(executable) = files::executable() else {
        return Value::Null;
    };
    json!({
        "claude": files::generated_state(Some(files::claude_marketplace_dir()), &executable),
        "codex_skill": files::generated_state(files::codex_skill_dir(), &executable),
    })
}
