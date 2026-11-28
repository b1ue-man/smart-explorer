//! Linux and Android facts of the mask lists: system configuration of the
//! agent hosts (Claude Code managed settings under `/etc/claude-code`,
//! Codex system configuration under `/etc/codex`; reference
//! docs/refs/agenten-sperre-elevation-2026-11-28.md §1.7 and §2.1). Paths
//! are case-sensitive.
use std::path::PathBuf;

pub(super) const CASE_INSENSITIVE_PATHS: bool = false;

pub(super) fn system_agent_config_dirs() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/etc/claude-code"),
        PathBuf::from("/etc/codex"),
    ]
}

/// Text markers without a file path (none on Linux).
pub(super) fn text_markers() -> Vec<(&'static str, PathBuf)> {
    Vec::new()
}
