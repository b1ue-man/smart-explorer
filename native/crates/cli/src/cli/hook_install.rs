//! Hook definitions that call `se hook`, shared by the plugin files
//! (`plugin/claude_hooks.rs`, `plugin/codex_hooks.rs`) and the managed
//! settings (`hook_managed.rs`), so every place registers the same command.
//!
//! - Claude Code runs a `command` handler through a shell (`sh -c` on Linux and
//!   macOS, `shell: "powershell"` chosen explicitly on Windows; CC-hooks „Exec
//!   form and shell form“). The command first checks that the launcher still
//!   exists and lets the call through when it does not, so an uninstalled se
//!   never blocks the host; with the launcher present `onFailure: "block"`
//!   turns every failure of the hook into a refusal (CC-hooks §1.8).
//! - Codex runs `command` through `$SHELL -lc` and `commandWindows` through
//!   `%COMSPEC% /C` (CX-src `hooks/src/engine/command_runner.rs`); its hooks are
//!   fail-open, so a missing launcher already lets the call through.
//! - Codex runs a non-managed hook only when `[hooks.state."<key>"]
//!   trusted_hash` equals the hash of its normalized definition (CX-src
//!   `hooks/src/engine/discovery.rs` `hook_hash`, `codex-config`
//!   `version_for_toml`): SHA-256 over the key-sorted compact JSON of
//!   `{event_name, matcher, hooks: [handler]}`, prefixed `sha256:`.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

/// The program the hooks run: the installed CLI of a managed installation
/// (`<root>/se`, stable across updates and removed with the installation;
/// the MCP launcher accepts only `mcp`), otherwise the registered `se` itself.
pub(in crate::cli) fn hook_program(registered: &Path) -> std::path::PathBuf {
    match crate::mcp_launcher::Installation::for_image(registered) {
        Ok(Some(installation)) => installation.installed_cli(),
        _ => registered.to_path_buf(),
    }
}

/// Seconds a host waits for `se hook` (it answers in milliseconds).
pub(in crate::cli) const TIMEOUT_SECONDS: u64 = 30;
/// Status line Claude Code shows while the hook runs.
const STATUS: &str = "Smart Explorer: checking the mask list lockdown";

fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn ps_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// The Claude Code command handler for `PreToolUse` and
/// `PreSkillShellCommand`.
pub(in crate::cli) fn claude_handler(executable: &Path, windows: bool) -> Value {
    let exe = executable.to_string_lossy();
    if windows {
        let exe = ps_quote(&exe);
        json!({
            "type": "command",
            "shell": "powershell",
            "command": format!("if (Test-Path -LiteralPath {exe} -PathType Leaf) {{ & {exe} hook claude; exit $LASTEXITCODE }} else {{ exit 0 }}"),
            "timeout": TIMEOUT_SECONDS,
            "onFailure": "block",
            "statusMessage": STATUS,
        })
    } else {
        let exe = sh_quote(&exe);
        json!({
            "type": "command",
            "command": format!("[ -x {exe} ] || exit 0; exec {exe} hook claude"),
            "timeout": TIMEOUT_SECONDS,
            "onFailure": "block",
            "statusMessage": STATUS,
        })
    }
}

/// The `hooks` object of a Claude Code settings or plugin hooks file: every
/// tool (`*`) and every skill shell command.
pub(in crate::cli) fn claude_hooks(executable: &Path, windows: bool) -> Value {
    let handler = claude_handler(executable, windows);
    json!({
        "PreToolUse": [{"matcher": "*", "hooks": [handler.clone()]}],
        "PreSkillShellCommand": [{"matcher": "*", "hooks": [handler]}],
    })
}

/// Codex `command` (Unix shell) and `commandWindows` (`cmd /C`).
pub(in crate::cli) fn codex_commands(executable: &Path) -> (String, String) {
    let exe = executable.to_string_lossy();
    (
        format!("{} hook codex", sh_quote(&exe)),
        format!("\"{exe}\" hook codex"),
    )
}

/// Matcher of the Codex group: every tool.
pub(in crate::cli) const CODEX_MATCHER: &str = "*";

/// Codex trust hash of the group with the command the running platform
/// executes (`commandWindows` on Windows, `command` elsewhere).
pub(in crate::cli) fn codex_trust_hash(effective_command: &str) -> String {
    let identity = json!({
        "event_name": "pre_tool_use",
        "hooks": [{
            "async": false,
            "command": effective_command,
            "timeout": TIMEOUT_SECONDS,
            "type": "command",
        }],
        "matcher": CODEX_MATCHER,
    });
    let bytes = serde_json::to_vec(&sorted(identity)).unwrap_or_default();
    let digest = Sha256::digest(bytes);
    format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

/// Key-sorted copy (serde_json may preserve insertion order).
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, sorted(value)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

/// Codex state key of handler 0 of group `group_index` in the PreToolUse list
/// of the file at `source` (CX-src `hooks/src/lib.rs` `hook_key`).
pub(in crate::cli) fn codex_state_key(source: &Path, group_index: usize) -> String {
    format!("{}:pre_tool_use:{group_index}:0", source.display())
}

/// Builds the Codex `[[hooks.PreToolUse]]` group as a TOML table.
pub(in crate::cli) fn codex_group(executable: &Path) -> toml_edit::Table {
    let (command, command_windows) = codex_commands(executable);
    let mut handler = toml_edit::Table::new();
    handler.insert("type", toml_edit::value("command"));
    handler.insert("command", toml_edit::value(command));
    handler.insert("commandWindows", toml_edit::value(command_windows));
    handler.insert("timeout", toml_edit::value(TIMEOUT_SECONDS as i64));
    let mut handlers = toml_edit::ArrayOfTables::new();
    handlers.push(handler);
    let mut group = toml_edit::Table::new();
    group.insert("matcher", toml_edit::value(CODEX_MATCHER));
    group.insert("hooks", toml_edit::Item::ArrayOfTables(handlers));
    group
}

/// Canonical text of a Codex group for ownership comparison: matcher and the
/// first handler's type, command, commandWindows and timeout.
pub(in crate::cli) fn codex_group_fingerprint(group: &toml_edit::Table) -> Option<String> {
    let handlers = group.get("hooks")?.as_array_of_tables()?;
    if handlers.len() != 1 {
        return None;
    }
    let handler = handlers.get(0)?;
    if handler.len() != 4 || group.len() != 2 {
        return None;
    }
    let text = |table: &toml_edit::Table, key: &str| {
        table
            .get(key)
            .and_then(toml_edit::Item::as_str)
            .map(str::to_owned)
    };
    let owned = json!({
        "matcher": text(group, "matcher")?,
        "type": text(handler, "type")?,
        "command": text(handler, "command")?,
        "commandWindows": text(handler, "commandWindows")?,
        "timeout": handler.get("timeout")?.as_integer()?,
    });
    let digest = Sha256::digest(serde_json::to_vec(&owned).ok()?);
    Some(format!("{digest:x}"))
}

/// `command` of the first handler of a Codex group.
pub(in crate::cli) fn codex_group_command(group: &toml_edit::Table) -> Option<&str> {
    group
        .get("hooks")
        .and_then(toml_edit::Item::as_array_of_tables)
        .and_then(|handlers| handlers.get(0))
        .and_then(|handler| handler.get("command"))
        .and_then(toml_edit::Item::as_str)
}

/// Whether `group` has exactly the shape se writes and runs `… hook codex`
/// (any se path): such a group is ours even when its recorded fingerprint
/// was lost.
pub(in crate::cli) fn is_se_codex_group(group: &toml_edit::Table) -> bool {
    codex_group_fingerprint(group).is_some()
        && codex_group_command(group).is_some_and(|command| command.ends_with(" hook codex"))
}
