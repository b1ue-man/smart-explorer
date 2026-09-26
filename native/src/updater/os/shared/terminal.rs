//! `se update`: feed check, installation layout, and the in-place
//! replacement of a terminal-only `se`.
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::config::update_source_str;
use super::core::{copy_file_checked, parse_ver, unique_sibling, verify_sha256};
use super::feed::classify_feed;
use super::feed_files::read_feed_version;
use super::os;
use super::staging::{remove_staged_path, stage_from_feed};
use super::types::StagedUpdate;

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
        .ok_or_else(|| format!("Installationsordner unbekannt: {}", cli.display()))?
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

/// Downloads and verifies app, helper and `se` for the desktop helper, the
/// same staging the app's update check produces.
pub fn stage_update(version: &str) -> Result<StagedUpdate, String> {
    let source = update_source_str().ok_or("Keine Update-Quelle konfiguriert")?;
    stage_from_feed(&classify_feed(&source), version)
}

/// A replaced terminal-only `se`. The previous file stays beside it until
/// the new one has proven that it starts.
#[must_use = "commit or roll back the replaced se"]
pub struct ReplacedCli {
    target: PathBuf,
    backup: PathBuf,
    sha256: String,
}

impl ReplacedCli {
    pub fn target(&self) -> &Path {
        &self.target
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn commit(self) -> Result<(), String> {
        std::fs::remove_file(&self.backup)
            .map_err(|error| format!("Sicherung {} entfernen: {error}", self.backup.display()))
    }

    /// Puts the previous file back with one atomic rename.
    pub fn rollback(self) -> Result<(), String> {
        std::fs::rename(&self.backup, &self.target)
            .map_err(|error| format!("Vorheriges se wiederherstellen: {error}"))
    }
}

/// Replaces the installed terminal-only `se` with the feed's `se` payload.
pub fn replace_cli(cli: &Path, version: &str) -> Result<ReplacedCli, String> {
    os::cli_self_replacement()?;
    let source = update_source_str().ok_or("Keine Update-Quelle konfiguriert")?;
    let payload = classify_feed(&source).fetch_cli_exe(version)?;
    let replaced = install_cli_in_place(payload.path(), payload.sha256(), cli);
    remove_staged_path(payload.path());
    replaced
}

/// Copies the verified payload beside `target` with the installed file's
/// permissions, verifies that copy again and renames it over `target`, so the
/// name never disappears and never refers to unverified bytes.
pub(super) fn install_cli_in_place(
    staged: &Path,
    sha256: &str,
    target: &Path,
) -> Result<ReplacedCli, String> {
    verify_sha256(staged, sha256)?;
    let permissions = std::fs::metadata(target)
        .map_err(|error| format!("Installiertes se {} lesen: {error}", target.display()))?
        .permissions();
    let backup = unique_sibling(target, "update-old");
    copy_file_checked(target, &backup, "Installiertes se sichern", None)?;
    let pending = unique_sibling(target, "update-pending");
    let swapped = copy_file_checked(staged, &pending, "Neues se", Some(sha256))
        .and_then(|()| {
            std::fs::set_permissions(&pending, permissions)
                .map_err(|error| format!("Rechte fuer neues se setzen: {error}"))
        })
        .and_then(|()| verify_sha256(&pending, sha256))
        .and_then(|()| {
            std::fs::rename(&pending, target)
                .map_err(|error| format!("Neues se einsetzen ({}): {error}", target.display()))
        });
    if let Err(error) = swapped {
        let _ = std::fs::remove_file(&pending);
        let _ = std::fs::remove_file(&backup);
        return Err(error);
    }
    Ok(ReplacedCli {
        target: target.to_path_buf(),
        backup,
        sha256: sha256.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::{install_cli_in_place, installation_in, Installation};

    fn names(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn file_sha256(path: &std::path::Path) -> String {
        super::super::core::sha256_file(path).unwrap()
    }

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

    #[test]
    fn cli_task_install_replaces_in_place_commits_and_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("se");
        std::fs::write(&target, b"old se").unwrap();
        let staged = dir.path().join("staged");
        std::fs::write(&staged, b"new se").unwrap();
        let sha256 = file_sha256(&staged);

        let replaced = install_cli_in_place(&staged, &sha256, &target).unwrap();
        assert_eq!(replaced.target(), target.as_path());
        assert_eq!(replaced.sha256(), sha256);
        assert_eq!(std::fs::read(&target).unwrap(), b"new se");
        replaced.rollback().unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"old se");
        assert_eq!(names(dir.path()), ["se", "staged"]);

        let replaced = install_cli_in_place(&staged, &sha256, &target).unwrap();
        replaced.commit().unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new se");
        assert_eq!(names(dir.path()), ["se", "staged"]);
    }

    #[test]
    fn cli_task_install_rejects_a_hash_mismatch_before_replacing() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("se");
        std::fs::write(&target, b"old se").unwrap();
        let staged = dir.path().join("staged");
        std::fs::write(&staged, b"tampered").unwrap();
        let expected = "0".repeat(64);
        assert!(install_cli_in_place(&staged, &expected, &target).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"old se");
        // The mismatching download is discarded and nothing else is left behind.
        assert_eq!(names(dir.path()), ["se"]);
    }
}
