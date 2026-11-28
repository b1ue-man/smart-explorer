//! Foreign MCP servers the user allows during the agent lockdown
//! (`se masks servers`, desktop page „Maskenlisten“). The list holds no secret,
//! so the hook reads it without the store key; it lives beside the generated
//! plugin files in `<app data>/ai-plugins/lockdown-servers.json`, which the
//! se tools refuse to write like every se app data file. A missing or damaged
//! file means: no foreign server is allowed (fail-closed). Allowing a server
//! weakens the lockdown and needs the user's confirmation through the system
//! dialog (`masks_cli.rs`, `settings_masks.rs`).
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const FILE: &str = "ai-plugins/lockdown-servers.json";

pub(in crate::cli) fn path() -> PathBuf {
    crate::support_dirs::app_data_dir().join(FILE)
}

/// Allowed server names, sorted and without duplicates.
pub(in crate::cli) fn allowed() -> Vec<String> {
    read_at(&path())
}

pub(in crate::cli) fn read_at(path: &Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return Vec::new();
    };
    if value["schema"] != 1 {
        return Vec::new();
    }
    let mut names: Vec<String> = value["allowed_mcp_servers"]
        .as_array()
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .filter(|name| valid(name).is_ok())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names.dedup();
    names
}

/// A server name as both hosts configure it: letters, digits, `_`, `-`, `.`,
/// `:` (Claude Code plugin servers are named `plugin:<plugin>:<server>`).
pub(in crate::cli) fn valid(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 200 {
        return Err("MCP-Servername braucht 1 bis 200 Zeichen".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
    {
        return Err("MCP-Servername nur aus Buchstaben, Ziffern, _, -, . und :".into());
    }
    if super::hook_policy::is_se_server_name(name) {
        return Err("Smart Explorer selbst ist immer erlaubt".into());
    }
    Ok(())
}

/// Writes the list atomically (staged file in the same folder).
pub(in crate::cli) fn write_at(path: &Path, names: &[String]) -> Result<(), String> {
    let mut names: Vec<String> = names.to_vec();
    names.sort();
    names.dedup();
    let parent = path.parent().ok_or("lockdown_servers_parent_missing")?;
    crate::support_dirs::ensure_private_dir(parent).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(&json!({"schema":1,"allowed_mcp_servers":names}))
        .map_err(|e| e.to_string())?;
    let mut staged = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    std::io::Write::write_all(&mut staged, &bytes)
        .and_then(|()| staged.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    staged.persist(path).map_err(|e| e.error.to_string())?;
    Ok(())
}

/// Adds (`allow = true`) or removes one name; returns the new list.
pub(in crate::cli) fn change(name: &str, allow: bool) -> Result<Vec<String>, String> {
    valid(name)?;
    let path = path();
    let mut names = read_at(&path);
    if allow {
        names.push(name.to_string());
    } else {
        names.retain(|entry| entry != name);
    }
    write_at(&path, &names)?;
    Ok(read_at(&path))
}
