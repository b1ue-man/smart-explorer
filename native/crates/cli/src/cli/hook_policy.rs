//! Decision of one agent host tool call while the agent lockdown of the mask
//! lists is on (plan 2026-11-28 S1, docs/refs/agenten-sperre-elevation-2026-11-28.md
//! §1.2, §2.2): the se tools and tools without file or process access stay,
//! every native file, shell, notebook and LSP tool, the output of background
//! shells and every foreign MCP tool except the servers the user allowed are
//! refused, web fetches to loopback, private addresses and `file:` as well.
//! Unknown tools are refused (fail-closed), so a new host tool never slips
//! through.
use serde_json::Value;

/// The agent host whose hook input is decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(super) enum HookHost {
    Claude,
    Codex,
}

/// Outcome of one decision: no output (the host's own permission flow
/// continues) or a refusal with the reason the agent reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Decision {
    Allow,
    Deny(String),
}

/// Claude Code names the plugin's tools `mcp__plugin_<plugin>_<server>__<tool>`
/// (`plugin/claude.rs`, report field `tools`).
pub(super) const CLAUDE_SE_PREFIX: &str = "mcp__plugin_smart-explorer_se__";
/// Codex names them `mcp__<server>__<tool>` with the server key of
/// `plugin/codex.rs` (`[mcp_servers.smart-explorer]`).
pub(super) const CODEX_SE_PREFIX: &str = "mcp__smart-explorer__";
/// Server names the MCP resource tools of both hosts use for se.
const SE_SERVER_NAMES: [&str; 3] = [
    "plugin:smart-explorer:se",
    "plugin_smart-explorer_se",
    "smart-explorer",
];

/// Claude Code tools without file or process access (CC-tools, CC-hooks).
/// `REPL` (Code Mode) only orchestrates calls, each of which reaches this hook
/// under its own name (CC-codemode „Permission checks“).
const CLAUDE_SAFE: [&str; 23] = [
    "Agent",
    "Task",
    "AskUserQuestion",
    "TodoWrite",
    "TaskCreate",
    "TaskUpdate",
    "TaskList",
    "TaskGet",
    "TaskStop",
    "KillShell",
    "KillBash",
    "ExitPlanMode",
    "EnterPlanMode",
    "WebSearch",
    "Skill",
    "ToolSearch",
    "SlashCommand",
    "EndConversation",
    "CronCreate",
    "CronDelete",
    "CronList",
    "SendMessage",
    "REPL",
];

/// Codex local function tools without file or process access (CX-src
/// `core/src/tools/spec_plan.rs`; hosted `web_search` never reaches a hook).
const CODEX_SAFE: [&str; 8] = [
    "update_plan",
    "spawn_agent",
    "Agent",
    "send_input",
    "wait",
    "close_agent",
    "resume_agent",
    "request_user_input",
];

/// The se tool an agent uses instead of a refused native tool.
fn instead(host: HookHost, tool: &str) -> String {
    let se = |name: &str| match host {
        HookHost::Claude => format!("{CLAUDE_SE_PREFIX}{name}"),
        HookHost::Codex => format!("{CODEX_SE_PREFIX}{name}"),
    };
    let tools: &[&str] = match tool {
        "Read" | "NotebookRead" => &["read"],
        "Write" => &["write"],
        "Edit" | "MultiEdit" | "NotebookEdit" => &["edit", "apply_patch"],
        "apply_patch" => &["apply_patch", "edit"],
        "Glob" | "LS" => &["search", "list"],
        "Grep" => &["grep"],
        "LSP" => &["grep", "read"],
        "Bash" | "PowerShell" | "Monitor" | "exec_command" | "shell" => &["shell"],
        "BashOutput" | "TaskOutput" => &["job_status"],
        _ => &["list", "read", "grep", "edit", "shell"],
    };
    tools
        .iter()
        .map(|name| se(name))
        .collect::<Vec<_>>()
        .join(", ")
}

fn lockdown_reason(host: HookHost, tool: &str) -> String {
    format!(
        "Smart Explorer agent lockdown: a mask list protects folders on this device, so `{tool}` is blocked for every path. Use the Smart Explorer tools instead ({}); they mask protected values in every answer.",
        instead(host, tool)
    )
}

/// Decides one `PreToolUse` input of `host` while the lockdown is on.
/// `allowed_servers` are the foreign MCP servers the user allowed (names as
/// configured in the host).
pub(super) fn decide_tool(host: HookHost, input: &Value, allowed_servers: &[String]) -> Decision {
    let Some(tool) = input.get("tool_name").and_then(Value::as_str) else {
        return Decision::Deny(
            "Smart Explorer agent lockdown: the tool call names no tool; it is blocked.".into(),
        );
    };
    let tool_input = input.get("tool_input").unwrap_or(&Value::Null);
    if let Some(rest) = tool.strip_prefix("mcp__") {
        return decide_mcp(host, tool, rest, input, allowed_servers);
    }
    let safe: &[&str] = match host {
        HookHost::Claude => &CLAUDE_SAFE,
        HookHost::Codex => &CODEX_SAFE,
    };
    if safe.contains(&tool) {
        return Decision::Allow;
    }
    match tool {
        "WebFetch" => match tool_input.get("url").and_then(Value::as_str) {
            Some(url) => match super::hook_web::check_url(url) {
                Ok(()) => Decision::Allow,
                Err(why) => Decision::Deny(format!(
                    "Smart Explorer agent lockdown: WebFetch of `{url}` is blocked ({why}); only public web addresses may be fetched while a mask list protects folders."
                )),
            },
            None => Decision::Deny("Smart Explorer agent lockdown: WebFetch without `url` is blocked.".into()),
        },
        "ListMcpResourcesTool" | "ReadMcpResourceTool" | "list_mcp_resources"
        | "list_mcp_resource_templates" | "read_mcp_resource" => {
            let server = tool_input.get("server").and_then(Value::as_str);
            match server {
                Some(server) if is_se_server(server) || server_allowed(server, allowed_servers) => {
                    Decision::Allow
                }
                _ => Decision::Deny(format!(
                    "Smart Explorer agent lockdown: MCP resources of server `{}` are blocked; only Smart Explorer and the servers the user allowed (`se masks servers`) are reachable.",
                    server.unwrap_or("(all servers)")
                )),
            }
        }
        _ => Decision::Deny(lockdown_reason(host, tool)),
    }
}

fn decide_mcp(
    host: HookHost,
    tool: &str,
    rest: &str,
    input: &Value,
    allowed_servers: &[String],
) -> Decision {
    let source = input
        .get("mcp_server")
        .and_then(|server| server.get("source"))
        .and_then(Value::as_str);
    let own = match host {
        // A user or project server that happens to carry the plugin's name is
        // not the plugin (CC-hooks „Match MCP tools“: trust by `source`).
        HookHost::Claude => {
            tool.starts_with(CLAUDE_SE_PREFIX) && source.is_none_or(|s| s == "plugin")
        }
        HookHost::Codex => tool.starts_with(CODEX_SE_PREFIX),
    };
    if own {
        return Decision::Allow;
    }
    let server = input
        .get("mcp_server")
        .and_then(|server| server.get("name"))
        .and_then(Value::as_str);
    let by_name = rest
        .split_once("__")
        .map(|(server, _)| server)
        .unwrap_or(rest);
    if server.is_some_and(|name| server_allowed(name, allowed_servers))
        || server_allowed(by_name, allowed_servers)
    {
        return Decision::Allow;
    }
    Decision::Deny(format!(
        "Smart Explorer agent lockdown: the MCP tool `{tool}` of another server is blocked while a mask list protects folders; the user can allow a server with `se masks servers allow <name>`. Use the Smart Explorer tools instead ({}).",
        instead(host, tool)
    ))
}

fn is_se_server(name: &str) -> bool {
    SE_SERVER_NAMES.contains(&name)
}

/// Whether `name` is one of the names under which the hosts know se.
pub(super) fn is_se_server_name(name: &str) -> bool {
    is_se_server(name)
}

fn server_allowed(name: &str, allowed: &[String]) -> bool {
    allowed.iter().any(|entry| entry == name)
}

/// Reason for a refused skill shell command (`PreSkillShellCommand`).
pub(super) fn skill_shell_reason() -> String {
    "Smart Explorer agent lockdown: shell commands embedded in skills are blocked while a mask list protects folders; run commands through the Smart Explorer `shell` tool.".into()
}

/// Reason while the mask state cannot be read (fail-closed).
pub(super) fn unknown_state_reason() -> String {
    "Smart Explorer agent lockdown: the mask list state cannot be read, so every tool except the Smart Explorer tools is blocked until it can (fail-closed).".into()
}
