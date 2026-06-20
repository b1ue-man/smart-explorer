use std::path::{Path, PathBuf};

use super::config::appdata_dir;
use super::core::parse_ver;

#[cfg(windows)]
fn binary_suffix() -> &'static str {
    ".exe"
}

#[cfg(target_os = "linux")]
fn binary_suffix() -> &'static str {
    ""
}

fn is_archived_binary(path: &Path) -> bool {
    #[cfg(windows)]
    {
        path.extension().and_then(|x| x.to_str()) == Some("exe")
    }
    #[cfg(target_os = "linux")]
    {
        path.is_file()
    }
}

fn archived_name_without_binary_suffix(path: &Path) -> Option<&str> {
    #[cfg(windows)]
    {
        path.file_stem().and_then(|s| s.to_str())
    }
    #[cfg(target_os = "linux")]
    {
        path.file_name().and_then(|s| s.to_str())
    }
}

/// Filename prefix for the renamed-out running binary (`<stem>_old`).
pub(super) fn old_binary_prefix(cur_exe: &Path) -> String {
    let stem = cur_exe
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "smart_explorer".into());
    format!("{}_old", stem)
}

/// A unique path to rename the running binary to.
pub(super) fn new_old_binary_path(cur_exe: &Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    cur_exe.with_file_name(format!(
        "{}_{}{}",
        old_binary_prefix(cur_exe),
        nanos,
        binary_suffix()
    ))
}

/// Delete leftovers from previous updates (best effort, with retries since an
/// old process may still hold one).
pub fn cleanup_old_binaries() {
    std::thread::Builder::new()
        .name("update-cleanup".into())
        .spawn(|| {
            let exe = match std::env::current_exe() {
                Ok(e) => e,
                Err(_) => return,
            };
            let dir = match exe.parent() {
                Some(d) => d.to_path_buf(),
                None => return,
            };
            let prefix = old_binary_prefix(&exe);
            for _ in 0..10 {
                let mut any_left = false;
                if let Ok(rd) = std::fs::read_dir(&dir) {
                    for e in rd.flatten() {
                        let name = e.file_name().to_string_lossy().to_string();
                        if name.starts_with(&prefix)
                            && name.ends_with(binary_suffix())
                            && std::fs::remove_file(e.path()).is_err()
                        {
                            any_left = true;
                        }
                    }
                }
                if !any_left {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        })
        .ok();
}

pub(super) fn versions_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|d| d.join("versions"))
}

fn pin_path() -> PathBuf {
    appdata_dir().join("update_pinned.txt")
}

/// Auto-update on launch is paused (the user reverted to an older version).
pub fn is_auto_update_paused() -> bool {
    pin_path().exists()
}

/// The version we're pinned to, if any.
pub fn pinned_version() -> Option<String> {
    std::fs::read_to_string(pin_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub(super) fn set_pin(version: &str) {
    let _ = std::fs::write(pin_path(), version);
}

/// Resume automatic updates (clears the rollback pin).
pub fn resume_auto_update() {
    let _ = std::fs::remove_file(pin_path());
}

pub(super) fn exe_stem(cur_exe: &Path) -> String {
    cur_exe
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Smart Explorer".into())
}

/// Copy the currently-running binary into the versions archive, labelled with
/// `version`. Best-effort; never fails the caller.
pub(super) fn archive_binary(version: &str) {
    let vd = match versions_dir() {
        Some(d) => d,
        None => return,
    };
    let _ = std::fs::create_dir_all(&vd);
    if let Ok(cur) = std::env::current_exe() {
        let dest = vd.join(format!("{} {}{}", exe_stem(&cur), version, binary_suffix()));
        if !dest.exists() {
            let _ = std::fs::copy(&cur, &dest);
        }
    }
}

/// Preserve the currently-running binary in the versions archive so it can be
/// rolled back to after a future update.
pub fn archive_current_version() {
    std::thread::Builder::new()
        .name("version-archive".into())
        .spawn(|| archive_binary(env!("CARGO_PKG_VERSION")))
        .ok();
}

/// Archived versions available to roll back to, newest first.
pub fn list_archived_versions() -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    if let Some(vd) = versions_dir() {
        if let Ok(rd) = std::fs::read_dir(&vd) {
            for e in rd.flatten() {
                let p = e.path();
                if !is_archived_binary(&p) {
                    continue;
                }
                if let Some(name) = archived_name_without_binary_suffix(&p) {
                    if let Some(ver) = name.rsplit(' ').next() {
                        if ver
                            .chars()
                            .next()
                            .map(|c| c.is_ascii_digit())
                            .unwrap_or(false)
                        {
                            out.push((ver.to_string(), p.clone()));
                        }
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| parse_ver(&b.0).cmp(&parse_ver(&a.0)));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}
