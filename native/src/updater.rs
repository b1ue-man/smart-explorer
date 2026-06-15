// Self-update against a local (or UNC) update feed.
//
// Feed layout (a plain folder):
//   <feed>/version.txt          — e.g. "0.2.1" (first line)
//   <feed>/Smart Explorer.exe   — the new binary
//
// Feed location resolution, first hit wins:
//   1. %APPDATA%\smart_explorer\update_source.txt   (user override, editable in the UI)
//   2. update_source.txt next to the running exe    (written by the installer)
//
// Update mechanics ("rename dance" — works on a running exe without admin
// rights as long as the install dir is user-writable, which it is for our
// per-user install under %LOCALAPPDATA%\Programs):
//   1. copy  <feed>/exe          → <app>/<stem>_update_pending.exe
//   2. rename <app>/<exe>        → <app>/<stem>_old.exe     (allowed while running)
//   3. rename pending            → <app>/<exe>
//   4. record applied version (loop protection for broken feeds)
//   5. caller relaunches the new exe with --updated and exits
// On the next start `cleanup_old_binaries` deletes the *_old.exe leftover.

use crossbeam_channel::Sender;
use std::path::PathBuf;

pub enum UpdateMsg {
    /// Feed reachable, no newer version. Only sent for manual checks.
    UpToDate { feed_version: String },
    /// No feed configured. Only sent for manual checks.
    NoFeed,
    /// New binary swapped in — relaunch `exe` with --updated and exit.
    Applied { version: String, exe: PathBuf },
    /// Only sent for manual checks; automatic checks fail silently.
    Error(String),
}

fn appdata_dir() -> PathBuf {
    let dir = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let app = dir.join("smart_explorer");
    let _ = std::fs::create_dir_all(&app);
    app
}

fn override_path() -> PathBuf {
    appdata_dir().join("update_source.txt")
}

fn last_applied_path() -> PathBuf {
    appdata_dir().join("last_applied_update.txt")
}

/// Resolve the configured update feed folder, if any.
pub fn update_source() -> Option<PathBuf> {
    let read_feed = |p: &PathBuf| -> Option<PathBuf> {
        let s = std::fs::read_to_string(p).ok()?;
        let line = s.lines().next()?.trim();
        if line.is_empty() {
            None
        } else {
            Some(PathBuf::from(line))
        }
    };
    if let Some(p) = read_feed(&override_path()) {
        return Some(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if let Some(p) = read_feed(&dir.join("update_source.txt")) {
                return Some(p);
            }
        }
    }
    None
}

/// Persist a user-chosen feed folder (empty string removes the override).
pub fn set_update_source(path: &str) -> std::io::Result<()> {
    let path = path.trim();
    if path.is_empty() {
        let _ = std::fs::remove_file(override_path());
        Ok(())
    } else {
        std::fs::write(override_path(), path)
    }
}

fn parse_ver(s: &str) -> (u64, u64, u64) {
    let mut it = s.trim().trim_start_matches('v').split('.');
    let mut next = || -> u64 {
        it.next()
            .and_then(|x| x.trim().parse::<u64>().ok())
            .unwrap_or(0)
    };
    (next(), next(), next())
}

fn old_binary_path(cur_exe: &std::path::Path) -> PathBuf {
    let stem = cur_exe
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "smart_explorer".into());
    cur_exe.with_file_name(format!("{}_old.exe", stem))
}

/// Delete leftovers from a previous update (best effort, with retries since
/// the old process may still be exiting).
pub fn cleanup_old_binaries() {
    std::thread::Builder::new()
        .name("update-cleanup".into())
        .spawn(|| {
            let exe = match std::env::current_exe() {
                Ok(e) => e,
                Err(_) => return,
            };
            let old = old_binary_path(&exe);
            if !old.exists() {
                return;
            }
            for _ in 0..10 {
                if std::fs::remove_file(&old).is_ok() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        })
        .ok();
}

// ─── Rollback support ────────────────────────────────────────────────────
//
// On every forward update we archive the OUTGOING binary into <app>/versions/
// as "Smart Explorer <version>.exe". The user can later revert to any archived
// version. A revert writes a pin file so the auto-updater doesn't immediately
// jump forward again; "update to latest" clears the pin.

fn versions_dir() -> Option<PathBuf> {
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

fn set_pin(version: &str) {
    let _ = std::fs::write(pin_path(), version);
}

/// Resume automatic updates (clears the rollback pin).
pub fn resume_auto_update() {
    let _ = std::fs::remove_file(pin_path());
}

fn exe_stem(cur_exe: &std::path::Path) -> String {
    cur_exe
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Smart Explorer".into())
}

/// Copy the currently-running binary into the versions archive, labelled with
/// `version`. Best-effort; never fails the caller.
fn archive_binary(version: &str) {
    let vd = match versions_dir() {
        Some(d) => d,
        None => return,
    };
    let _ = std::fs::create_dir_all(&vd);
    if let Ok(cur) = std::env::current_exe() {
        let dest = vd.join(format!("{} {}.exe", exe_stem(&cur), version));
        if !dest.exists() {
            let _ = std::fs::copy(&cur, &dest);
        }
    }
}

/// Preserve the currently-running binary in the versions archive so it can be
/// rolled back to after a future update — regardless of how we got here (e.g.
/// a jump from a pre-rollback version straight to the latest). Runs on a
/// background thread; best-effort.
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
                if p.extension().and_then(|x| x.to_str()) != Some("exe") {
                    continue;
                }
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    // "Smart Explorer <ver>" -> last whitespace token is the version
                    if let Some(ver) = stem.rsplit(' ').next() {
                        if ver.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
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

/// The "rename dance" that swaps `new_exe` into the running binary's path.
/// Returns the path the caller should relaunch with `--updated`.
fn swap_in(new_exe: &std::path::Path) -> Result<PathBuf, String> {
    let cur_exe = std::env::current_exe().map_err(|e| format!("Eigener Pfad unbekannt: {}", e))?;
    let stem = exe_stem(&cur_exe);
    let pending = cur_exe.with_file_name(format!("{}_update_pending.exe", stem));
    let old = old_binary_path(&cur_exe);

    std::fs::copy(new_exe, &pending).map_err(|e| format!("Kopieren fehlgeschlagen: {}", e))?;
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&cur_exe, &old).map_err(|e| {
        let _ = std::fs::remove_file(&pending);
        format!("Programmdatei kann nicht ersetzt werden ({}): {}", cur_exe.display(), e)
    })?;
    if let Err(e) = std::fs::rename(&pending, &cur_exe) {
        let _ = std::fs::rename(&old, &cur_exe);
        let _ = std::fs::remove_file(&pending);
        return Err(format!("Einsetzen fehlgeschlagen: {}", e));
    }
    Ok(cur_exe)
}

/// Revert to an archived binary. Archives the version we're leaving (so the
/// user can go forward again), swaps the old binary in, and pins so the
/// auto-updater won't undo the rollback. Returns the exe to relaunch.
pub fn revert_to(archived: &std::path::Path, version: &str) -> Result<PathBuf, String> {
    if !archived.exists() {
        return Err("Archivierte Version nicht gefunden".into());
    }
    archive_binary(env!("CARGO_PKG_VERSION"));
    let cur_exe = swap_in(archived)?;
    set_pin(version);
    Ok(cur_exe)
}

/// Force a forward update to the feed's latest, clearing any rollback pin.
/// Runs on its own thread; result via `tx`.
pub fn update_to_latest_async(tx: Sender<UpdateMsg>) {
    std::thread::Builder::new()
        .name("update-latest".into())
        .spawn(move || {
            resume_auto_update();
            match check_and_apply(true) {
                Ok(Some(msg)) => {
                    let _ = tx.send(msg);
                }
                Ok(None) => {}
                Err(e) => {
                    let _ = tx.send(UpdateMsg::Error(e));
                }
            }
        })
        .ok();
}

/// Check the feed and, if it carries a newer version, swap the binary in
/// place. Runs on its own thread; result arrives via `tx`.
/// `manual` = user clicked "check now" (gets feedback even for no-op results).
pub fn check_async(tx: Sender<UpdateMsg>, manual: bool) {
    std::thread::Builder::new()
        .name("updater".into())
        .spawn(move || {
            let result = check_and_apply(manual);
            match result {
                Ok(Some(msg)) => {
                    let _ = tx.send(msg);
                }
                Ok(None) => {}
                Err(e) => {
                    if manual {
                        let _ = tx.send(UpdateMsg::Error(e));
                    }
                }
            }
        })
        .ok();
}

fn check_and_apply(manual: bool) -> Result<Option<UpdateMsg>, String> {
    // Don't auto-jump forward while the user has pinned an older version.
    // (update_to_latest_async clears the pin before calling this with manual.)
    if !manual && is_auto_update_paused() {
        return Ok(None);
    }
    let feed = match update_source() {
        Some(f) => f,
        None => {
            return Ok(if manual { Some(UpdateMsg::NoFeed) } else { None });
        }
    };

    let feed_version = std::fs::read_to_string(feed.join("version.txt"))
        .map_err(|e| format!("Update-Feed nicht lesbar ({}): {}", feed.display(), e))?
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if feed_version.is_empty() {
        return Err(format!("version.txt im Feed {} ist leer", feed.display()));
    }

    let current = env!("CARGO_PKG_VERSION");
    if parse_ver(&feed_version) <= parse_ver(current) {
        return Ok(if manual {
            Some(UpdateMsg::UpToDate { feed_version })
        } else {
            None
        });
    }

    // Loop protection: if we already applied this exact feed version but our
    // version didn't change, the feed binary is mislabeled — don't reapply
    // forever.
    if let Ok(last) = std::fs::read_to_string(last_applied_path()) {
        if last.trim() == feed_version {
            return Err(format!(
                "Update {} wurde bereits angewendet, aber die Programmversion ist weiterhin {} — version.txt im Feed passt nicht zur EXE",
                feed_version, current
            ));
        }
    }

    // Locate the new binary in the feed
    let candidates = ["Smart Explorer.exe", "smart_explorer.exe"];
    let new_exe = candidates
        .iter()
        .map(|n| feed.join(n))
        .find(|p| p.exists())
        .ok_or_else(|| format!("Keine EXE im Update-Feed {} gefunden", feed.display()))?;

    // Archive the version we're replacing so it can be rolled back to, then
    // swap in the new binary.
    archive_binary(current);
    let cur_exe = swap_in(&new_exe)?;

    // We're now on the latest — clear any rollback pin and record the version.
    resume_auto_update();
    let _ = std::fs::write(last_applied_path(), &feed_version);
    Ok(Some(UpdateMsg::Applied {
        version: feed_version,
        exe: cur_exe,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archived_versions_parse_and_sort_numerically() {
        let vd = versions_dir().expect("versions dir");
        std::fs::create_dir_all(&vd).unwrap();
        let mk = ["0.3.6", "0.3.10", "0.4.0"];
        for v in mk {
            std::fs::write(vd.join(format!("Smart Explorer {}.exe", v)), b"x").unwrap();
        }
        let vers: Vec<String> = list_archived_versions().into_iter().map(|(v, _)| v).collect();
        let idx = |s: &str| vers.iter().position(|x| x == s);
        assert!(idx("0.4.0").is_some() && idx("0.3.10").is_some() && idx("0.3.6").is_some());
        // Numeric (not lexical) ordering, newest first: 0.4.0 > 0.3.10 > 0.3.6.
        assert!(idx("0.4.0") < idx("0.3.10"));
        assert!(idx("0.3.10") < idx("0.3.6"));
        for v in mk {
            let _ = std::fs::remove_file(vd.join(format!("Smart Explorer {}.exe", v)));
        }
    }

    #[test]
    fn pin_roundtrip() {
        let had = pinned_version();
        set_pin("0.3.6");
        assert!(is_auto_update_paused());
        assert_eq!(pinned_version().as_deref(), Some("0.3.6"));
        resume_auto_update();
        assert!(!is_auto_update_paused());
        if let Some(v) = had {
            set_pin(&v); // restore any pre-existing pin
        }
    }
}
