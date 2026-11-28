//! Guarded paths (plan S1, K3): the se app data (with the mask store and
//! the secret store) and the configuration of the agent hosts that carries
//! the lockdown hooks (Claude Code settings, plugins and managed settings,
//! Codex `config.toml`, `requirements.toml` and hooks). se tools refuse
//! them for agents; the se shell refuses commands that name them.
//! `refers_to_guarded` reads command text: it is a best-effort word check
//! (quotes removed, separators unified), the integrity guard stays the
//! enforcement for the mask store.
use std::path::{Path, PathBuf};

use super::platform;

/// Environment variables whose value may prefix a guarded path in a
/// command (`$HOME/.claude`, `%APPDATA%\smart_explorer`, …).
const PATH_VARIABLES: &[&str] = &[
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "XDG_DATA_HOME",
    "CLAUDE_CONFIG_DIR",
    "CODEX_HOME",
    "PROGRAMDATA",
    "PROGRAMFILES",
];

/// Project-level host settings that can switch hooks off or change them
/// (`disableAllHooks`, Codex hooks); matched wherever they appear.
const HOST_SETTING_TAILS: &[&str] = &[
    ".claude/settings.json",
    ".claude/settings.local.json",
    ".claude.json",
    ".codex/config.toml",
    ".codex/hooks.json",
    ".codex/requirements.toml",
];

/// Every guarded path of this user and host.
pub fn guarded_paths() -> Vec<PathBuf> {
    let mut paths = vec![crate::support_dirs::app_data_dir()];
    let home = crate::support_dirs::home_dir();
    match env_path("CLAUDE_CONFIG_DIR") {
        Some(dir) => {
            paths.push(dir.join(".claude.json"));
            paths.push(dir);
        }
        None => {
            if let Some(home) = &home {
                paths.push(home.join(".claude"));
            }
        }
    }
    if let Some(home) = &home {
        paths.push(home.join(".claude.json"));
    }
    match env_path("CODEX_HOME") {
        Some(dir) => paths.push(dir),
        None => {
            if let Some(home) = &home {
                paths.push(home.join(".codex"));
            }
        }
    }
    paths.extend(platform::system_agent_config_dirs());
    paths.sort();
    paths.dedup();
    paths
}

/// The guarded path a command line or path text names, if any: absolute
/// forms, forms behind `~` and environment variables, the names `.claude`,
/// `.claude.json`, `.codex` as a path component anywhere (project settings
/// too), `smart_explorer` next to a separator, and per-OS markers (the
/// Claude Code policy key in the Windows registry).
pub fn refers_to_guarded(text: &str) -> Option<PathBuf> {
    let variants = [normalize(text), normalize(&strip_quotes(text))];
    let guarded = guarded_paths();
    for path in &guarded {
        for form in forms(path) {
            if variants.iter().any(|v| contains_bounded(v, &form, false)) {
                return Some(path.clone());
            }
        }
    }
    for tail in HOST_SETTING_TAILS {
        if variants
            .iter()
            .any(|v| contains_bounded(v, &fold_case(tail), false))
        {
            return Some(PathBuf::from(tail));
        }
    }
    let app = fold_case("smart_explorer");
    if variants.iter().any(|v| contains_bounded(v, &app, true)) {
        return Some(crate::support_dirs::app_data_dir());
    }
    for (marker, path) in platform::text_markers() {
        if variants.iter().any(|v| v.contains(&fold_case(marker))) {
            return Some(path);
        }
    }
    None
}

/// A guarded path that `path` lies in, or that lies in `path` (a walk of a
/// parent folder would reach it). Lexical and, where the path exists,
/// after resolving links.
pub fn guarded_overlap(path: &Path) -> Option<PathBuf> {
    let candidates = [Some(path.to_path_buf()), resolve_existing(path)];
    for guarded in guarded_paths() {
        let guarded_forms = [Some(guarded.clone()), resolve_existing(&guarded)];
        for candidate in candidates.iter().flatten() {
            for form in guarded_forms.iter().flatten() {
                let a = normalize(&candidate.to_string_lossy());
                let b = normalize(&form.to_string_lossy());
                if is_within(&a, &b) || is_within(&b, &a) {
                    return Some(guarded.clone());
                }
            }
        }
    }
    None
}

fn is_within(inner: &str, outer: &str) -> bool {
    let outer = outer.trim_end_matches('/');
    inner == outer || inner.starts_with(&format!("{outer}/"))
}

/// The deepest existing ancestor resolved through links, with the rest
/// appended.
fn resolve_existing(path: &Path) -> Option<PathBuf> {
    let mut rest = Vec::new();
    let mut current = path;
    loop {
        if let Ok(resolved) = std::fs::canonicalize(current) {
            let mut out = resolved;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return Some(out);
        }
        rest.push(current.file_name()?.to_os_string());
        current = current.parent()?;
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The written forms of `path`: absolute and behind `~` or a variable.
fn forms(path: &Path) -> Vec<String> {
    let absolute = normalize(&path.to_string_lossy());
    let mut forms = vec![absolute.clone()];
    if let Some(home) = crate::support_dirs::home_dir() {
        let home = normalize(&home.to_string_lossy());
        if let Some(rest) = strip_dir(&absolute, &home) {
            forms.push(format!("~{rest}"));
        }
    }
    for name in PATH_VARIABLES {
        let Some(value) = env_path(name) else {
            continue;
        };
        let value = normalize(&value.to_string_lossy());
        let Some(rest) = strip_dir(&absolute, &value) else {
            continue;
        };
        for prefix in [
            format!("${name}"),
            format!("${{{name}}}"),
            format!("%{name}%"),
            format!("$env:{name}"),
        ] {
            forms.push(format!("{}{rest}", fold_case(&prefix)));
        }
    }
    forms
}

/// `/rest` when `path` is `dir` or below it.
fn strip_dir<'a>(path: &'a str, dir: &str) -> Option<&'a str> {
    let dir = dir.trim_end_matches('/');
    if dir.is_empty() {
        return None;
    }
    let rest = path.strip_prefix(dir)?;
    (rest.is_empty() || rest.starts_with('/')).then_some(rest)
}

fn fold_case(text: &str) -> String {
    if platform::CASE_INSENSITIVE_PATHS {
        text.to_lowercase()
    } else {
        text.to_string()
    }
}

/// Separators unified to `/`, repeated `/` and `/./` collapsed, case folded
/// where the file system ignores case.
pub(crate) fn normalize(text: &str) -> String {
    let unified: String = text
        .chars()
        .map(|c| if c == '\\' { '/' } else { c })
        .collect();
    let mut out = String::with_capacity(unified.len());
    for c in fold_case(&unified).chars() {
        if c == '/' && out.ends_with('/') {
            continue;
        }
        out.push(c);
        if out.ends_with("/./") {
            out.truncate(out.len() - 2);
        }
    }
    out
}

fn strip_quotes(text: &str) -> String {
    text.chars()
        .filter(|c| !matches!(c, '"' | '\'' | '`'))
        .collect()
}

fn is_path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.')
}

/// `needle` occurs in `hay` as a whole path piece: no path character right
/// before it (a `/` before is allowed only for relative names, which start
/// with `.` or are `smart_explorer`), none right after (a `/` after is
/// fine). With `need_separator`, a `/` must touch one side.
fn contains_bounded(hay: &str, needle: &str, need_separator: bool) -> bool {
    if needle.is_empty() {
        return false;
    }
    let absolute = needle.starts_with('/')
        || needle.starts_with('~')
        || needle.starts_with('$')
        || needle.starts_with('%')
        || needle.as_bytes().get(1) == Some(&b':');
    let mut from = 0;
    while let Some(found) = hay[from..].find(needle) {
        let start = from + found;
        let end = start + needle.len();
        let before = hay[..start].chars().next_back();
        let after = hay[end..].chars().next();
        let before_ok = match before {
            None => true,
            Some('/') => !absolute,
            Some(c) => !is_path_char(c),
        };
        let after_ok = after.is_none_or(|c| !is_path_char(c));
        let touches_separator = before == Some('/') || after == Some('/');
        if before_ok && after_ok && (!need_separator || touches_separator) {
            return true;
        }
        from = start + needle.len().max(1);
        while !hay.is_char_boundary(from) {
            from += 1;
        }
    }
    false
}
