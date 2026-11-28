//! `se mcp`: the Model Context Protocol server through which AI coding agents
//! (Claude Code, Codex, any MCP client) use Smart Explorer: files on every
//! saved remote, Share peer, Google Drive and the local disk (also below a
//! working root kept open like a tab), transfers, name and content search,
//! remote commands on saved SSH remotes and authorized Share peers
//! (also in persistent shell sessions that terminals can join), and every
//! other `se` command. Agent hosts start it over stdio; `se plugin`
//! installs it. Design and host contracts: docs/AI_AGENT_PLUGIN.md.
#[path = "mcp/apps_tool.rs"]
mod apps_tool;
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
/// Output records of content searches, shared with `se grep`.
#[path = "mcp/grep_records.rs"]
pub(super) mod grep_records;
#[path = "mcp/grep_tool.rs"]
mod grep_tool;
#[path = "mcp/job_view.rs"]
mod job_view;
#[path = "mcp/jobs.rs"]
mod jobs;
#[path = "mcp/line_read.rs"]
mod line_read;
#[path = "mcp/login_shutdown.rs"]
mod login_shutdown;
#[path = "mcp/login_tool.rs"]
mod login_tool;
#[path = "mcp/mask_layer.rs"]
mod mask_layer;
#[path = "mcp/mask_layer_guard.rs"]
mod mask_layer_guard;
#[path = "mcp/mask_layer_read.rs"]
mod mask_layer_read;
#[path = "mcp/mask_layer_stream.rs"]
mod mask_layer_stream;
#[path = "mcp/masks_tool.rs"]
mod masks_tool;
#[path = "mcp/os/mod.rs"]
mod os;
#[path = "mcp/patch_tool.rs"]
mod patch_tool;
#[path = "mcp/rdp_shutdown.rs"]
mod rdp_shutdown;
#[path = "mcp/rdp_tool.rs"]
mod rdp_tool;
#[path = "mcp/replace_tool.rs"]
mod replace_tool;
#[path = "mcp/response.rs"]
mod response;
#[path = "mcp/root_path.rs"]
mod root_path;
#[path = "mcp/root_tool.rs"]
mod root_tool;
#[path = "mcp/screen_tool.rs"]
mod screen_tool;
#[path = "mcp/screen_window_tool.rs"]
mod screen_window_tool;
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
#[path = "mcp/shell_tool.rs"]
mod shell_tool;
#[path = "mcp/shell_tool_guard.rs"]
mod shell_tool_guard;
#[path = "mcp/shell_tool_local.rs"]
mod shell_tool_local;
#[path = "mcp/shell_tool_shells.rs"]
mod shell_tool_shells;
#[cfg(test)]
#[path = "mcp/shell_tool_tests.rs"]
mod shell_tool_tests;
#[path = "mcp/spool.rs"]
mod spool;
#[path = "mcp/status_tools.rs"]
mod status_tools;
#[path = "mcp/tab_cache.rs"]
mod tab_cache;
#[path = "mcp/targets.rs"]
mod targets;
#[path = "mcp/tcp_tool.rs"]
mod tcp_tool;
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
copy, move, name search, content search with grep) for local paths, saved
remotes (@label:/path and saved endpoints), Share peers and Google Drive, with
a working root (root) that keeps one target open like an explorer tab and
takes paths relative to it; remote commands (exec) on saved SFTP
remotes over SSH and on Share peers that granted this device Exec, also in
persistent shell sessions (exec with session) that `se shell --attach` can
join, and the `sessions` tool that joins a `se shell --attachable`; the
`screen` tool that lists the screens and windows of paired devices and
returns a still picture of one; jobs for long work; and the `se` tool for
every other se command. Standard output
carries only the protocol; diagnostics go to standard error. The server ends
when its client closes standard input or a Smart Explorer update starts; its
own sessions end with it.";

#[derive(clap::Args)]
#[command(long_about = MCP_HELP)]
pub(super) struct McpArgs {}

pub(super) fn run(_args: McpArgs) -> Result<i32, String> {
    if let Some(code) = crate::mcp_launcher::launch_from_installed()? {
        return Ok(code);
    }
    server::serve()
}
