//! `se update`: feed check, installation layout, and the in-place
//! replacement of a terminal-only `se`.
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::cli_swap::{
    arm_interrupt_undo, install_cli_in_place, remove_orphans, ReplacedCli, UpdateLock,
};
use super::config::update_source_str;
use super::core::parse_ver;
use super::feed::classify_feed;
use super::feed_files::read_feed_version;
use super::os;
use super::staging::{remove_staged_path, stage_from_feed};
use super::types::StagedUpdate;

const NO_SOURCE: &str = "no update source is configured";

#[derive(Clone, Debug, Serialize)]
pub struct FeedCheck {
    pub source: String,
    pub current_version: String,
    pub feed_version: String,
    pub update_available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Installation {
    /// Only `se`: it replaces its own installed file.
    TerminalOnly { cli: PathBuf },
    /// `se` beside the desktop app: the hash-bound helper replaces app,
    /// helper and `se` together, as the app's own update does.
    Desktop { cli: PathBuf, app: PathBuf },
}

impl Installation {
    pub fn cli(&self) -> &Path {
        match self {
            Self::TerminalOnly { cli } | Self::Desktop { cli, .. } => cli,
        }
    }
}

/// The configured feed's version compared with this build; `None` without
/// a configured update source.
pub fn check_feed() -> Result<Option<FeedCheck>, String> {
    let Some(source) = update_source_str() else {
        return Ok(None);
    };
    let feed_version = read_feed_version(&source)?;
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let update_available = parse_ver(&feed_version) > parse_ver(&current_version);
    Ok(Some(FeedCheck {
        source,
        current_version,
        feed_version,
        update_available,
    }))
}

pub fn detect_installation() -> Result<Installation, String> {
    let cli = os::running_cli_path()?;
    let dir = cli
        .parent()
        .ok_or_else(|| format!("{} has no installation folder", cli.display()))?
        .to_path_buf();
    Ok(installation_in(&dir, cli))
}

fn installation_in(dir: &Path, cli: PathBuf) -> Installation {
    let app = os::installed_app_names()
        .iter()
        .map(|name| dir.join(name))
        .find(|app| app.is_file());
    match app {
        Some(app) => Installation::Desktop { cli, app },
        None => Installation::TerminalOnly { cli },
    }
}

/// The desktop helper restarts Smart Explorer after replacing it; without a
/// graphical session that restart fails and the helper rolls everything back.
pub fn desktop_update_possible() -> Result<(), String> {
    os::desktop_session()
}

/// Downloads and verifies app, helper and `se` for the desktop helper, the
/// same staging the app's update check produces.
pub fn stage_update(version: &str) -> Result<StagedUpdate, String> {
    let source = update_source_str().ok_or(NO_SOURCE)?;
    stage_from_feed(&classify_feed(&source), version)
}

/// Replaces the installed terminal-only `se` with the feed's `se` payload.
pub fn replace_cli(cli: &Path, version: &str) -> Result<ReplacedCli, String> {
    os::cli_self_replacement()?;
    let lock = UpdateLock::acquire(cli)?;
    remove_orphans(cli);
    arm_interrupt_undo()?;
    let source = update_source_str().ok_or(NO_SOURCE)?;
    let payload = classify_feed(&source).fetch_cli_exe(version)?;
    let replaced = install_cli_in_place(payload.path(), payload.sha256(), cli, lock);
    remove_staged_path(payload.path());
    replaced
}

#[cfg(test)]
mod tests {
    use super::{installation_in, Installation};

    #[test]
    fn cli_task_installation_detects_terminal_only_and_desktop() {
        let dir = tempfile::tempdir().unwrap();
        let cli = dir.path().join("se");
        std::fs::write(&cli, b"se").unwrap();
        assert_eq!(
            installation_in(dir.path(), cli.clone()),
            Installation::TerminalOnly { cli: cli.clone() }
        );
        let app = dir.path().join(super::os::installed_app_names()[0]);
        std::fs::write(&app, b"app").unwrap();
        let desktop = installation_in(dir.path(), cli.clone());
        assert_eq!(desktop.cli(), cli.as_path());
        assert_eq!(desktop, Installation::Desktop { cli, app });
    }
}
