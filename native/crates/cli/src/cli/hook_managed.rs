//! Hardening of the agent lockdown through the hosts' managed settings, which
//! the agent cannot change and user settings cannot switch off
//! (docs/refs/agenten-sperre-elevation-2026-11-28.md §1.6, §1.9, §2.1):
//!
//! - Claude Code: the drop-in `managed-settings.d/90-smart-explorer.json` in
//!   the system folder (`/etc/claude-code/`, `/Library/Application
//!   Support/ClaudeCode/`, `C:\Program Files\ClaudeCode\`; CC-managed
//!   „File-based“, merged after `managed-settings.json`, lists combined) with
//!   the `se hook claude` hook and `Read(//…)`/`Edit(//…)` deny rules for the
//!   protected local folders and the se app data (they also cover
//!   @-mentions and IDE context, which reach no hook).
//! - Codex: one `[[hooks.PreToolUse]]` group in `requirements.toml`
//!   (`/etc/codex/`, `%ProgramData%\OpenAI\Codex\`); managed hooks count as
//!   trusted.
//!
//! Writing there needs administrator rights: `se elevated managed-hooks`
//! (`run_elevated`) runs elevated through `crate::elevate`. The unelevated
//! side compares the wanted content with the files (world-readable) and
//! elevates only when they differ, so the system dialog appears on the first
//! activation of a list with a protected folder and when the protected
//! folders change.
use super::hook_install;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const CLAUDE_FILE: &str = "90-smart-explorer.json";

/// What the elevated helper writes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::cli) struct ManagedRequest {
    /// The se program the hooks run (`hook_install::hook_program`).
    pub(in crate::cli) launcher: PathBuf,
    /// Absolute local paths denied to the hosts' own file tools.
    pub(in crate::cli) deny_paths: Vec<PathBuf>,
    pub(in crate::cli) claude: bool,
    pub(in crate::cli) codex: bool,
}

fn program_files() -> PathBuf {
    std::env::var_os("ProgramFiles")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
}

fn program_data() -> PathBuf {
    std::env::var_os("ProgramData")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
}

/// The Claude Code drop-in file of this platform.
pub(in crate::cli) fn claude_dropin() -> PathBuf {
    let folder = if cfg!(windows) {
        program_files().join("ClaudeCode")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/ClaudeCode")
    } else {
        PathBuf::from("/etc/claude-code")
    };
    folder.join("managed-settings.d").join(CLAUDE_FILE)
}

/// The Codex system `requirements.toml` of this platform.
pub(in crate::cli) fn codex_requirements() -> PathBuf {
    if cfg!(windows) {
        program_data().join(r"OpenAI\Codex\requirements.toml")
    } else {
        PathBuf::from("/etc/codex/requirements.toml")
    }
}

/// A Claude Code permission path: `//` + the absolute path in POSIX form
/// (`C:\x` → `//c/x`, CC-perm „How path patterns match“).
pub(in crate::cli) fn rule_path(path: &Path) -> Option<String> {
    let text = path.to_string_lossy().replace('\\', "/");
    let bytes = text.as_bytes();
    let posix = if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        format!("/{}{}", (bytes[0] as char).to_ascii_lowercase(), &text[2..])
    } else if text.starts_with('/') {
        text.clone()
    } else {
        return None;
    };
    let posix = posix.trim_end_matches('/');
    Some(format!("/{}", if posix.is_empty() { "/" } else { posix }))
}

/// The deny rules for `paths`: the path itself and everything below it, for
/// `Read` (also Grep, Glob, LSP, Write, Edit, mentions) and `Edit` (also
/// NotebookEdit).
pub(in crate::cli) fn deny_rules(paths: &[PathBuf]) -> Vec<String> {
    let mut rules = Vec::new();
    for path in paths {
        let Some(rule) = rule_path(path) else {
            continue;
        };
        let below = if rule.ends_with('/') {
            format!("{rule}**")
        } else {
            format!("{rule}/**")
        };
        for tool in ["Read", "Edit"] {
            rules.push(format!("{tool}({rule})"));
            rules.push(format!("{tool}({below})"));
        }
    }
    rules.sort();
    rules.dedup();
    rules
}

/// The whole Claude Code drop-in document.
pub(in crate::cli) fn claude_document(request: &ManagedRequest, windows: bool) -> Value {
    json!({
        "hooks": hook_install::claude_hooks(&request.launcher, windows),
        "permissions": {"deny": deny_rules(&request.deny_paths)},
    })
}

/// `requirements.toml` with the se group (replacing an older se group);
/// every other key, table, comment and hook stays.
pub(in crate::cli) fn codex_document(
    before: Option<&str>,
    launcher: &Path,
) -> Result<String, String> {
    let mut doc = parse_toml(before)?;
    if doc.get("hooks").is_none() {
        let mut table = toml_edit::Table::new();
        table.set_implicit(true);
        doc["hooks"] = toml_edit::Item::Table(table);
    }
    let hooks = doc["hooks"]
        .as_table_mut()
        .ok_or("integration_conflict: requirements.toml `hooks` is not a table")?;
    if hooks.get("PreToolUse").is_none() {
        hooks.insert(
            "PreToolUse",
            toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new()),
        );
    }
    let groups = hooks["PreToolUse"].as_array_of_tables_mut().ok_or(
        "integration_conflict: requirements.toml `hooks.PreToolUse` is not an array of tables",
    )?;
    groups.retain(|group| !hook_install::is_se_codex_group(group));
    groups.push(hook_install::codex_group(launcher));
    Ok(doc.to_string())
}

/// `requirements.toml` without the se group; `None` when nothing else is left.
pub(in crate::cli) fn codex_without_se(before: &str) -> Result<Option<String>, String> {
    let mut doc = parse_toml(Some(before))?;
    if let Some(hooks) = doc.get_mut("hooks").and_then(toml_edit::Item::as_table_mut) {
        if let Some(groups) = hooks
            .get_mut("PreToolUse")
            .and_then(toml_edit::Item::as_array_of_tables_mut)
        {
            groups.retain(|group| !hook_install::is_se_codex_group(group));
            if groups.is_empty() {
                hooks.remove("PreToolUse");
            }
        }
        if hooks.is_empty() {
            doc.remove("hooks");
        }
    }
    let text = doc.to_string();
    Ok((!text.trim().is_empty()).then_some(text))
}

fn parse_toml(text: Option<&str>) -> Result<toml_edit::DocumentMut, String> {
    text.unwrap_or_default()
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| {
            format!("integration_conflict: requirements.toml is not valid TOML: {error}")
        })
}

fn codex_state(text: &str, launcher: &Path) -> &'static str {
    let Ok(doc) = parse_toml(Some(text)) else {
        return "conflict";
    };
    let groups = doc
        .get("hooks")
        .and_then(|hooks| hooks.get("PreToolUse"))
        .and_then(toml_edit::Item::as_array_of_tables);
    let wanted = hook_install::codex_group_fingerprint(&hook_install::codex_group(launcher));
    match groups.and_then(|groups| groups.iter().find(|g| hook_install::is_se_codex_group(g))) {
        Some(group) if hook_install::codex_group_fingerprint(group) == wanted => "current",
        Some(_) => "outdated",
        None => "absent",
    }
}

fn read_text(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| format!("{} is not UTF-8", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

/// State of each host's managed hardening for `request`: `current`,
/// `outdated`, `absent`, `unreadable` or `conflict` (invalid TOML).
pub(in crate::cli) fn state_for(request: &ManagedRequest) -> Value {
    let claude = match read_text(&claude_dropin()) {
        Ok(Some(text)) => match serde_json::from_str::<Value>(&text) {
            Ok(value) if value == claude_document(request, cfg!(windows)) => "current",
            _ => "outdated",
        },
        Ok(None) => "absent",
        Err(_) => "unreadable",
    };
    let codex = match read_text(&codex_requirements()) {
        Ok(Some(text)) => codex_state(&text, &request.launcher),
        Ok(None) => "absent",
        Err(_) => "unreadable",
    };
    json!({
        "claude": {"wanted": request.claude, "state": claude, "path": claude_dropin()},
        "codex": {"wanted": request.codex, "state": codex, "path": codex_requirements()},
    })
}

/// Writes `bytes` through a hidden staged sibling (Claude Code ignores hidden
/// files and names without `.json`), then renames it into place.
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("managed_parent_missing")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let staged = parent.join(format!(".{name}.{}.se-staged", std::process::id()));
    let result = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)
        .and_then(|mut file| file.write_all(bytes).and_then(|()| file.sync_all()))
        .and_then(|()| std::fs::rename(&staged, path));
    if let Err(error) = result {
        let _ = std::fs::remove_file(&staged);
        return Err(format!("{}: {error}", path.display()));
    }
    Ok(())
}

fn valid_request(request: &ManagedRequest) -> Result<(), String> {
    if !request.launcher.is_absolute() || !request.launcher.is_file() {
        return Err(format!(
            "managed_hooks_launcher_invalid: {}",
            request.launcher.display()
        ));
    }
    if let Some(path) = request
        .deny_paths
        .iter()
        .find(|path| rule_path(path).is_none())
    {
        return Err(format!(
            "managed_hooks_path_not_absolute: {}",
            path.display()
        ));
    }
    Ok(())
}

/// Applies `request` with the rights of the running process (elevated).
pub(in crate::cli) fn apply_now(request: &ManagedRequest) -> Result<Value, String> {
    valid_request(request)?;
    let mut done = serde_json::Map::new();
    if request.claude {
        let document = claude_document(request, cfg!(windows));
        let bytes = serde_json::to_vec_pretty(&document).map_err(|e| e.to_string())?;
        write_file(&claude_dropin(), &bytes)?;
        done.insert("claude".into(), json!(claude_dropin()));
    }
    if request.codex {
        let path = codex_requirements();
        let before = read_text(&path)?;
        let after = codex_document(before.as_deref(), &request.launcher)?;
        // Codex refuses to start on invalid requirements: parse what is written.
        parse_toml(Some(&after))?;
        if before.as_deref() != Some(after.as_str()) {
            write_file(&path, after.as_bytes())?;
        }
        done.insert("codex".into(), json!(path));
    }
    Ok(Value::Object(done))
}

/// Removes the se entries of the selected hosts (elevated).
pub(in crate::cli) fn remove_now(claude: bool, codex: bool) -> Result<Value, String> {
    let mut done = serde_json::Map::new();
    if claude {
        let path = claude_dropin();
        match std::fs::remove_file(&path) {
            Ok(()) => {
                done.insert("claude".into(), json!("removed"));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                done.insert("claude".into(), json!("absent"));
            }
            Err(error) => return Err(format!("{}: {error}", path.display())),
        }
    }
    if codex {
        let path = codex_requirements();
        let state = match read_text(&path)? {
            Some(before) => match codex_without_se(&before)? {
                Some(after) if after == before => "absent",
                Some(after) => {
                    write_file(&path, after.as_bytes())?;
                    "removed"
                }
                None => {
                    std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                    "removed"
                }
            },
            None => "absent",
        };
        done.insert("codex".into(), json!(state));
    }
    Ok(Value::Object(done))
}

/// Whether managed se entries exist for the hosts (decides whether a removal
/// needs the system dialog at all).
pub(in crate::cli) fn present(claude: bool, codex: bool) -> bool {
    (claude && claude_dropin().exists())
        || (codex
            && read_text(&codex_requirements())
                .ok()
                .flatten()
                .is_some_and(|text| codex_state(&text, Path::new("")) != "absent"))
}

/// `se elevated managed-hooks apply <hex json>` / `remove <claude|codex|all>`.
pub(in crate::cli) fn run_elevated(args: &[String]) -> Result<i32, String> {
    println!("{}", execute(args)?);
    Ok(0)
}

/// The action of `run_elevated` with the rights of the running process.
pub(in crate::cli) fn execute(args: &[String]) -> Result<Value, String> {
    Ok(match args.first().map(String::as_str) {
        Some("apply") => {
            let encoded = args.get(1).ok_or("managed_hooks_request_missing")?;
            apply_now(&decode_request(encoded)?)?
        }
        Some("remove") => {
            let (claude, codex) = match args.get(1).map(String::as_str) {
                Some("claude") => (true, false),
                Some("codex") => (false, true),
                Some("all") | None => (true, true),
                Some(other) => return Err(format!("managed_hooks_host_unknown: {other}")),
            };
            remove_now(claude, codex)?
        }
        other => return Err(format!("managed_hooks_action_unknown: {other:?}")),
    })
}

/// Hex of the request JSON: survives every command-line quoting of the
/// elevation paths (ShellExecuteEx parameters, pkexec argv).
pub(in crate::cli) fn encode_request(request: &ManagedRequest) -> Result<String, String> {
    let bytes = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(in crate::cli) fn decode_request(text: &str) -> Result<ManagedRequest, String> {
    if text.len() % 2 != 0 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("managed_hooks_request_invalid".into());
    }
    let bytes = (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16))
        .collect::<Result<Vec<u8>, _>>()
        .map_err(|_| "managed_hooks_request_invalid".to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| format!("managed_hooks_request_invalid: {e}"))
}
