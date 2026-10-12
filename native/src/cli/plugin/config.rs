//! `se plugin config <host>`: the configuration another MCP client needs,
//! with the absolute path of this `se` (formats checked 2026-10-12, see
//! docs/refs/codex-mcp-spec-rmcp-2026-10-12.md §1 and the host docs there).
use serde_json::json;
use std::path::Path;

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub(super) enum ConfigHost {
    /// `claude mcp add` (without the plugin's skill)
    Claude,
    /// `~/.codex/config.toml`
    Codex,
    /// `mcpServers` JSON: Cursor `~/.cursor/mcp.json`, Gemini CLI
    /// `~/.gemini/settings.json`, Antigravity, Claude Desktop
    Generic,
    /// VS Code `mcp.json` (`servers`)
    Vscode,
    /// OpenCode `opencode.json` (`mcp`)
    Opencode,
}

pub(super) fn render(host: ConfigHost, executable: &Path) -> String {
    let command = executable.to_string_lossy();
    match host {
        ConfigHost::Claude => format!(
            "claude mcp add --scope user --transport stdio smart-explorer -- {} mcp\n",
            shell_quote(&command)
        ),
        ConfigHost::Codex => format!(
            "[mcp_servers.smart-explorer]\ncommand = {}\nargs = [\"mcp\"]\n",
            toml_string(&command)
        ),
        ConfigHost::Generic => pretty(json!({
            "mcpServers": {"smart-explorer": {"command": command, "args": ["mcp"]}}
        })),
        ConfigHost::Vscode => pretty(json!({
            "servers": {"smart-explorer": {"type": "stdio", "command": command, "args": ["mcp"]}}
        })),
        ConfigHost::Opencode => pretty(json!({
            "mcp": {"smart-explorer": {"type": "local", "command": [command, "mcp"], "enabled": true}}
        })),
    }
}

fn pretty(value: serde_json::Value) -> String {
    serde_json::to_string_pretty(&value).unwrap_or_default() + "\n"
}

/// A TOML basic string (backslashes and quotes escaped).
fn toml_string(text: &str) -> String {
    let mut quoted = String::from("\"");
    for character in text.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            other => quoted.push(other),
        }
    }
    quoted.push('"');
    quoted
}

fn shell_quote(text: &str) -> String {
    if cfg!(windows) {
        format!("\"{text}\"")
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::{render, toml_string, ConfigHost};
    use std::path::Path;

    #[test]
    fn mcp_task_client_configs_carry_the_absolute_server_path() {
        assert_eq!(
            toml_string(r#"C:\Program Files\se "x".exe"#),
            r#""C:\\Program Files\\se \"x\".exe""#
        );
        let path = Path::new("/opt/smart explorer/se");
        let codex = render(ConfigHost::Codex, path);
        assert!(codex.contains("command = \"/opt/smart explorer/se\""));
        let generic: serde_json::Value =
            serde_json::from_str(&render(ConfigHost::Generic, path)).unwrap();
        assert_eq!(generic["mcpServers"]["smart-explorer"]["args"][0], "mcp");
        let vscode: serde_json::Value =
            serde_json::from_str(&render(ConfigHost::Vscode, path)).unwrap();
        assert_eq!(vscode["servers"]["smart-explorer"]["type"], "stdio");
        assert!(render(ConfigHost::Claude, path).contains("smart-explorer -- "));
    }
}
