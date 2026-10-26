//! Claude Code: a local marketplace with the Smart Explorer plugin, added and
//! installed through `claude plugin …` (docs/refs/claude-code-plugins-mcp-2026-10-12.md §2).
use super::files::{self, MARKETPLACE};
use super::host_cli;
use serde_json::{json, Value};
use std::path::Path;

pub(super) const PLUGIN_ID: &str = "smart-explorer@smart-explorer";

pub(super) fn install(executable: &Path) -> Result<Value, String> {
    let root = files::write_claude(executable)?;
    let root_text = root.to_string_lossy().into_owned();
    let Some(claude) = host_cli::find("claude") else {
        return Ok(json!({
            "host": "claude",
            "installed": false,
            "files": root,
            "error": "Claude Code (`claude`) was not found on PATH",
            "next": [
                format!("claude plugin marketplace add \"{root_text}\""),
                format!("claude plugin install {PLUGIN_ID}"),
            ],
        }));
    };
    // Never remove an existing host registration as an installation step.
    // A conflicting marketplace remains visible for explicit repair.
    let added = host_cli::run(
        &claude,
        &[
            "plugin",
            "marketplace",
            "add",
            root_text.as_str(),
            "--scope",
            "user",
        ],
    )?;
    if !added.success && !marketplace_is_ours(&claude, &root)? {
        return Err(format!(
            "`claude plugin marketplace add` failed ({})",
            added.detail()
        ));
    }
    let installed = host_cli::run(
        &claude,
        &["plugin", "install", PLUGIN_ID, "--scope", "user", "-y"],
    )?;
    if !installed.success {
        return Err(format!(
            "`claude plugin install {PLUGIN_ID}` failed ({})",
            installed.detail()
        ));
    }
    Ok(json!({
        "host": "claude",
        "installed": true,
        "cli": claude,
        "files": root,
        "tools": format!("mcp__plugin_smart-explorer_{}__<tool>", files::CLAUDE_SERVER),
        "next": "Start a new Claude Code session (or run /reload-plugins) to load the Smart Explorer tools.",
    }))
}

pub(super) fn uninstall() -> Result<Value, String> {
    let root = files::claude_marketplace_dir();
    if files::generated_files_absent(&root)? {return Ok(json!({"host":"claude","removed_files":false,"steps":[]}));}
    files::verify_owned(&root)?;
    let mut host = Vec::new();
    if let Some(claude) = host_cli::find("claude") {
        if !marketplace_is_ours(&claude, &root)? { return Err("integration_conflict: Claude marketplace points to an unowned source".into()); }
        for args in [
            &["plugin", "uninstall", PLUGIN_ID, "--scope", "user", "-y"][..],
            &["plugin", "marketplace", "remove", MARKETPLACE][..],
        ] {
            let output = host_cli::run(&claude, args)?;
            if !output.success {
                return Err(format!("Claude removal failed ({}); generated ownership files retained for retry",output.detail()));
            }
            host.push(json!({
                "command": format!("claude {}", args.join(" ")),
                "ok": output.success,
                "detail": output.detail(),
            }));
        }
    }
    let removed = files::remove_generated(&files::claude_marketplace_dir())?;
    Ok(json!({ "host": "claude", "removed_files": removed, "steps": host }))
}

pub(super) fn status(executable: &Path) -> Value {
    let cli = host_cli::find("claude");
    let listed = cli
        .as_ref()
        .and_then(|claude| host_cli::run(claude, &["plugin", "list", "--json"]).ok())
        .map(|output| output.success && output.stdout.contains(files::PLUGIN));
    json!({
        "host": "claude",
        "cli": cli,
        "plugin_listed": listed,
        "files": files::generated_state(Some(files::claude_marketplace_dir()), executable),
    })
}

fn marketplace_is_ours(cli:&std::path::Path,root:&Path)->Result<bool,String> {
    let listed=host_cli::run(cli,&["plugin","marketplace","list","--json"])?;
    if !listed.success {return Err(format!("cannot verify Claude marketplace ownership ({})",listed.detail()));}
    let value:Value=serde_json::from_str(&listed.stdout).map_err(|_|"integration_conflict: Claude marketplace listing schema unsupported")?;
    fn matches(value:&Value,root:&Path)->bool {
        match value {
            Value::Array(values)=>values.iter().any(|value|matches(value,root)),
            Value::Object(map)=> {
                let name=map.get("name").and_then(Value::as_str);
                let source=map.get("source").and_then(Value::as_str)
                    .or_else(||map.get("source").and_then(|s|s.get("path")).and_then(Value::as_str));
                (name==Some(MARKETPLACE) && source.is_some_and(|s|Path::new(s)==root))
                    || map.values().any(|value|matches(value,root))
            },
            _=>false,
        }
    }
    Ok(matches(&value,root))
}
