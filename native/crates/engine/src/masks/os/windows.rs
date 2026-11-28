//! Windows facts of the mask lists: system configuration of the agent
//! hosts (Claude Code managed settings in `%ProgramFiles%\ClaudeCode` and
//! the older `%ProgramData%\ClaudeCode`, the policy key
//! `HKLM\SOFTWARE\Policies\ClaudeCode` and its `HKCU` twin, Codex system
//! configuration in `%ProgramData%\OpenAI\Codex`; reference
//! docs/refs/agenten-sperre-elevation-2026-11-28.md §1.7 and §2.1). NTFS
//! names ignore case.
use std::path::PathBuf;

pub(super) const CASE_INSENSITIVE_PATHS: bool = true;

pub(super) fn system_agent_config_dirs() -> Vec<PathBuf> {
    let program_files = folder("ProgramFiles", r"C:\Program Files");
    let program_data = folder("ProgramData", r"C:\ProgramData");
    vec![
        program_files.join("ClaudeCode"),
        program_data.join("ClaudeCode"),
        program_data.join("OpenAI").join("Codex"),
    ]
}

/// The Claude Code policy key in the registry (both hives), as written in
/// `reg`/PowerShell commands with `\` unified to `/`.
pub(super) fn text_markers() -> Vec<(&'static str, PathBuf)> {
    vec![(
        "software/policies/claudecode",
        PathBuf::from(r"HKLM\SOFTWARE\Policies\ClaudeCode"),
    )]
}

fn folder(variable: &str, fallback: &str) -> PathBuf {
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(fallback))
}
