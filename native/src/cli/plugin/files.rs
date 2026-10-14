//! Files `se plugin` generates: a local Claude Code marketplace with the
//! Smart Explorer plugin (server entry with the absolute path of this `se`,
//! skill), and the Codex skill. Each generated directory carries a marker with
//! the version and executable it was written for; `se mcp` rewrites outdated
//! ones at start (Claude Code reloads local-marketplace plugins at every
//! session start), and uninstalling removes only marked directories.
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub(super) const SKILL: &str = include_str!("assets/SKILL.md");
pub(super) const PLUGIN: &str = "smart-explorer";
pub(super) const MARKETPLACE: &str = "smart-explorer";
/// Server name inside the Claude Code plugin; tools appear as
/// `mcp__plugin_smart-explorer_se__<tool>`.
pub(super) const CLAUDE_SERVER: &str = "se";
const MARKER: &str = ".se-generated.json";
const DESCRIPTION: &str = "Smart Explorer for AI agents, used before the built-in file tools: files on local disks, saved remotes (SFTP, FTP/FTPS, WebDAV, SMB), Share peers and Google Drive, and remote commands over SSH or Smart Explorer Share.";

pub(super) fn executable() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|error| format!("cannot determine this se executable: {error}"))
}

pub(super) fn claude_marketplace_dir() -> PathBuf {
    crate::support_dirs::app_data_dir()
        .join("ai-plugins")
        .join("claude")
}

/// The user-level skill directory Codex scans (`$HOME/.agents/skills`).
pub(super) fn codex_skill_dir() -> Option<PathBuf> {
    crate::support_dirs::home_dir().map(|home| home.join(".agents").join("skills").join(PLUGIN))
}

pub(super) fn write_claude(executable: &Path) -> Result<PathBuf, String> {
    let root = claude_marketplace_dir();
    let plugin = root.join(PLUGIN);
    write_json(
        &root.join(".claude-plugin").join("marketplace.json"),
        &json!({
            "name": MARKETPLACE,
            "owner": {"name": "Smart Explorer"},
            "plugins": [{
                "name": PLUGIN,
                "source": format!("./{PLUGIN}"),
                "description": DESCRIPTION,
            }],
        }),
    )?;
    write_json(
        &plugin.join(".claude-plugin").join("plugin.json"),
        &json!({
            "name": PLUGIN,
            "displayName": "Smart Explorer",
            "version": env!("CARGO_PKG_VERSION"),
            "description": DESCRIPTION,
            "author": {"name": "Smart Explorer"},
            "homepage": "https://github.com/b1ue-man/smart-explorer",
            "license": "MIT",
        }),
    )?;
    write_json(
        &plugin.join(".mcp.json"),
        &json!({
            "mcpServers": {
                CLAUDE_SERVER: {
                    "command": executable.to_string_lossy(),
                    "args": ["mcp"],
                },
            },
        }),
    )?;
    write_text(&plugin.join("skills").join(PLUGIN).join("SKILL.md"), SKILL)?;
    write_marker(&root, executable)?;
    Ok(root)
}

pub(super) fn write_codex_skill(executable: &Path) -> Result<PathBuf, String> {
    let directory =
        codex_skill_dir().ok_or_else(|| "no home directory for the Codex skill".to_string())?;
    if directory.exists() && read_marker(&directory).is_none() {
        return Err(format!(
            "{} exists and was not created by se; remove or rename it first",
            directory.display()
        ));
    }
    write_text(&directory.join("SKILL.md"), SKILL)?;
    write_marker(&directory, executable)?;
    Ok(directory)
}

/// Removes a generated directory; refuses directories without the marker.
pub(super) fn remove_generated(directory: &Path) -> Result<bool, String> {
    if !directory.exists() {
        return Ok(false);
    }
    if read_marker(directory).is_none() {
        return Err(format!(
            "{} was not created by se; left unchanged",
            directory.display()
        ));
    }
    std::fs::remove_dir_all(directory)
        .map(|()| true)
        .map_err(|error| format!("cannot remove {}: {error}", directory.display()))
}

/// Version and executable a generated directory was written for.
pub(super) fn read_marker(directory: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(directory.join(MARKER)).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_marker(directory: &Path, executable: &Path) -> Result<(), String> {
    write_json(
        &directory.join(MARKER),
        &json!({
            "version": env!("CARGO_PKG_VERSION"),
            "executable": executable.to_string_lossy(),
        }),
    )
}

/// Generated state for status reports: present, and current for this `se`.
pub(super) fn generated_state(directory: Option<PathBuf>, executable: &Path) -> Value {
    let Some(directory) = directory else {
        return json!({ "generated": false });
    };
    match read_marker(&directory) {
        None => json!({ "generated": false, "path": directory }),
        Some(marker) => json!({
            "generated": true,
            "path": directory,
            "version": marker["version"],
            "current": is_current(&marker, executable),
        }),
    }
}

fn is_current(marker: &Value, executable: &Path) -> bool {
    marker["version"] == env!("CARGO_PKG_VERSION")
        && marker["executable"].as_str() == Some(executable.to_string_lossy().as_ref())
}

/// Rewrites generated files written by another `se` version or path. Errors
/// only reach standard error: they must not stop the server.
pub(super) fn refresh() {
    let Ok(executable) = executable() else {
        return;
    };
    let claude = claude_marketplace_dir();
    if read_marker(&claude).is_some_and(|marker| !is_current(&marker, &executable)) {
        if let Err(error) = write_claude(&executable) {
            eprintln!("se mcp: cannot refresh the Claude Code plugin files: {error}");
        }
    }
    if let Some(codex) = codex_skill_dir() {
        if read_marker(&codex).is_some_and(|marker| !is_current(&marker, &executable)) {
            if let Err(error) = write_codex_skill(&executable) {
                eprintln!("se mcp: cannot refresh the Codex skill: {error}");
            }
        }
    }
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    write_text(path, &(text + "\n"))
}

/// Writes through a sibling temporary file and a rename, so a host reading
/// the plugin at session start never sees a half-written file.
fn write_text(path: &Path, text: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("cannot write in {}: {error}", parent.display()))?;
    std::io::Write::write_all(&mut staged, text.as_bytes())
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    staged
        .persist(path)
        .map(|_| ())
        .map_err(|error| format!("cannot write {}: {}", path.display(), error.error))
}

#[cfg(test)]
mod tests {
    use super::{is_current, SKILL};
    use serde_json::json;
    use std::path::Path;

    #[test]
    fn mcp_task_skill_frontmatter_fits_codex_and_claude() {
        let frontmatter = SKILL
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"))
            .map(|(head, _)| head)
            .expect("SKILL.md frontmatter");
        let field = |name: &str| {
            frontmatter
                .lines()
                .find_map(|line| line.strip_prefix(&format!("{name}: ")))
                .unwrap_or_default()
                .to_string()
        };
        assert_eq!(field("name"), "smart-explorer");
        let description = field("description");
        assert!(!description.is_empty() && description.len() <= 1_024);
        let marker = json!({"version": env!("CARGO_PKG_VERSION"), "executable": "/opt/se"});
        assert!(is_current(&marker, Path::new("/opt/se")));
        assert!(!is_current(&marker, Path::new("/usr/bin/se")));
    }
}
