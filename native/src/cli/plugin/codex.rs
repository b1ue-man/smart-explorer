//! Codex: the MCP server entry through `codex mcp add` (absolute path of this
//! `se`; an Agent Plugin's command would have to be a bare program name found
//! on PATH) and the skill in `$HOME/.agents/skills`
//! (docs/refs/codex-mcp-spec-rmcp-2026-10-12.md §1).
use super::files;
use super::host_cli;
use serde_json::{json, Value};
use std::path::Path;

pub(super) const SERVER: &str = "smart-explorer";

/// Variables Codex filters out of an MCP server's environment on Unix but
/// `se` needs to find the same app data (`XDG_DATA_HOME`) and to start the
/// background worker in the user session (`XDG_RUNTIME_DIR`,
/// `DBUS_SESSION_BUS_ADDRESS`); passed with their values at install time.
const PASSED_ENV: &[&str] = &[
    "XDG_DATA_HOME",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
];

pub(super) fn install(executable: &Path) -> Result<Value, String> {
    // A server entry is replaced only when se wrote it before (marker of an
    // earlier installation) or it already runs this executable.
    let installed_before = files::codex_skill_dir()
        .as_deref()
        .and_then(files::read_marker)
        .is_some();
    let skill = files::write_codex_skill(executable)?;
    let Some(codex) = host_cli::find("codex") else {
        return Ok(json!({
            "host": "codex",
            "installed": false,
            "skill": skill,
            "error": "Codex (`codex`) was not found on PATH",
            "next": "add the entry printed by `se plugin config codex` to ~/.codex/config.toml",
        }));
    };
    let executable_text = executable.to_string_lossy().into_owned();
    let existing = host_cli::run(&codex, &["mcp", "get", SERVER, "--json"])?;
    if existing.success {
        let ours = installed_before || existing.stdout.contains(&json_text(&executable_text));
        if !ours {
            return Err(format!(
                "Codex already has another MCP server named `{SERVER}`; remove it with `codex mcp remove {SERVER}` first"
            ));
        }
        let removed = host_cli::run(&codex, &["mcp", "remove", SERVER])?;
        if !removed.success {
            return Err(format!(
                "`codex mcp remove {SERVER}` failed ({})",
                removed.detail()
            ));
        }
    }
    let env = passed_env();
    let mut args = vec!["mcp".to_string(), "add".to_string(), SERVER.to_string()];
    for (name, value) in &env {
        args.push("--env".into());
        args.push(format!("{name}={value}"));
    }
    args.extend(["--".to_string(), executable_text.clone(), "mcp".to_string()]);
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    let added = host_cli::run(&codex, &refs)?;
    if !added.success {
        return Err(format!(
            "`codex mcp add {SERVER}` failed ({})",
            added.detail()
        ));
    }
    let verified = host_cli::run(&codex, &["mcp", "get", SERVER, "--json"])
        .map(|output| output.success && output.stdout.contains(&json_text(&executable_text)))
        .unwrap_or(false);
    Ok(json!({
        "host": "codex",
        "installed": true,
        "verified": verified,
        "cli": codex,
        "skill": skill,
        "env": env.iter().map(|(name, _)| name).collect::<Vec<_>>(),
        "next": "Start a new Codex session to load the Smart Explorer tools.",
    }))
}

pub(super) fn uninstall() -> Result<Value, String> {
    let mut steps = Vec::new();
    if let Some(codex) = host_cli::find("codex") {
        let output = host_cli::run(&codex, &["mcp", "remove", SERVER])?;
        steps.push(json!({
            "command": format!("codex mcp remove {SERVER}"),
            "ok": output.success,
            "detail": output.detail(),
        }));
    }
    let removed = match files::codex_skill_dir() {
        Some(directory) => files::remove_generated(&directory)?,
        None => false,
    };
    Ok(json!({ "host": "codex", "removed_skill": removed, "steps": steps }))
}

pub(super) fn status(executable: &Path) -> Value {
    let cli = host_cli::find("codex");
    let configured = cli.as_ref().map(|codex| {
        host_cli::run(codex, &["mcp", "get", SERVER, "--json"])
            .map(|output| {
                json!({
                    "present": output.success,
                    "current": output.success
                        && output.stdout.contains(&json_text(&executable.to_string_lossy())),
                })
            })
            .unwrap_or_else(|error| json!({ "error": error }))
    });
    json!({
        "host": "codex",
        "cli": cli,
        "server": configured,
        "skill": files::generated_state(files::codex_skill_dir(), executable),
    })
}

fn passed_env() -> Vec<(&'static str, String)> {
    if cfg!(windows) {
        // Codex passes APPDATA, USERPROFILE and LOCALAPPDATA on Windows.
        return Vec::new();
    }
    PASSED_ENV
        .iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.is_empty())
                .map(|value| (*name, value))
        })
        .collect()
}

/// How a path appears inside JSON output (backslashes escaped).
fn json_text(text: &str) -> String {
    serde_json::to_string(text)
        .map(|quoted| quoted.trim_matches('"').to_string())
        .unwrap_or_else(|_| text.to_string())
}
