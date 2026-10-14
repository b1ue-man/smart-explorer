//! `se mcp`: the Model Context Protocol server through which AI coding agents
//! (Claude Code, Codex, any MCP client) use Smart Explorer: files on every
//! saved remote, Share peer, Google Drive and the local disk, transfers, name
//! search, remote commands on saved SSH remotes and authorized Share peers
//! (also in persistent shell sessions that terminals can join), and every
//! other `se` command. Agent hosts start it over stdio; `se plugin`
//! installs it. Design and host contracts: docs/AI_AGENT_PLUGIN.md.
#[path = "mcp/args.rs"]
mod args;
#[path = "mcp/catalog.rs"]
mod catalog;
#[path = "mcp/cli_tool.rs"]
mod cli_tool;
#[path = "mcp/exec_session_job.rs"]
mod exec_session_job;
#[path = "mcp/exec_share_job.rs"]
mod exec_share_job;
#[path = "mcp/exec_ssh_job.rs"]
mod exec_ssh_job;
#[path = "mcp/exec_tool.rs"]
mod exec_tool;
#[path = "mcp/file_tools.rs"]
mod file_tools;
#[path = "mcp/file_write.rs"]
mod file_write;
#[path = "mcp/job_view.rs"]
mod job_view;
#[path = "mcp/jobs.rs"]
mod jobs;
#[path = "mcp/os/mod.rs"]
mod os;
#[path = "mcp/response.rs"]
mod response;
#[path = "mcp/search_tool.rs"]
mod search_tool;
#[path = "mcp/server.rs"]
mod server;
#[path = "mcp/session.rs"]
mod session;
#[path = "mcp/sessions_tool.rs"]
mod sessions_tool;
#[path = "mcp/shell_registry.rs"]
mod shell_registry;
#[path = "mcp/spool.rs"]
mod spool;
#[path = "mcp/status_tools.rs"]
mod status_tools;
#[path = "mcp/targets.rs"]
mod targets;
#[path = "mcp/transfer_tools.rs"]
mod transfer_tools;
#[path = "mcp/update_stop.rs"]
mod update_stop;
#[path = "mcp/wait.rs"]
mod wait;

const MCP_HELP: &str = "\
Serve Smart Explorer to an AI agent over the Model Context Protocol on
standard input/output. Agent hosts start this command themselves; install it
into Claude Code or Codex with `se plugin install`, or print the
configuration for another MCP client with `se plugin config`.

The server offers file tools (list, stat, read, write, edit, mkdir, remove,
copy, move, search) for local paths, saved remotes (@label:/path and saved
endpoints), Share peers and Google Drive; remote commands (exec) on saved SFTP
remotes over SSH and on Share peers that granted this device Exec, also in
persistent shell sessions (exec with session) that `se shell --attach` can
join, and the `sessions` tool that joins a `se shell --attachable`; jobs for
long work; and the `se` tool for every other se command. Standard output
carries only the protocol; diagnostics go to standard error. The server ends
when its client closes standard input or a Smart Explorer update starts; its
own sessions end with it.";

#[derive(clap::Args)]
#[command(long_about = MCP_HELP)]
pub(super) struct McpArgs {}

pub(super) fn run(_args: McpArgs) -> Result<i32, String> {
    server::serve()
}
