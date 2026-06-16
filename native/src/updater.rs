// Self-update against an update feed. The feed is EITHER a folder (local disk
// or `\\server\share` UNC) OR an http(s) URL — e.g. the project's own git repo
// served over raw.githubusercontent.com ("set the git as update location":
// point the source at `release-native/update-feed/` in the repo and every push
// publishes an update).
//
// Feed layout (identical for both transports):
//   <feed>/version.txt          — e.g. "0.3.9" (first line)
//   <feed>/smart_explorer.exe   — the new binary (also "Smart Explorer.exe")
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
use std::path::{Path, PathBuf};
use std::time::Duration;

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

/// The raw configured update source string (folder path OR http(s) URL),
/// first hit wins. Used by the UI text field and the transport classifier.
pub fn update_source_str() -> Option<String> {
    let read = |p: &PathBuf| -> Option<String> {
        let s = std::fs::read_to_string(p).ok()?;
        let line = s.lines().next()?.trim().to_string();
        if line.is_empty() {
            None
        } else {
            Some(line)
        }
    };
    if let Some(s) = read(&override_path()) {
        return Some(s);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if let Some(s) = read(&dir.join("update_source.txt")) {
                return Some(s);
            }
        }
    }
    None
}

// ─── Update transport: local folder feed OR http(s)/git feed ──────────────
//
// Both layouts are identical (`<base>/version.txt` + `<base>/smart_explorer.exe`);
// only the transport differs, so all the version-compare / rename-dance /
// rollback machinery below is shared. Adding a transport here never touches the
// update flow in `check_and_apply`.

const HTTP_TIMEOUT: Duration = Duration::from_secs(25);

enum Feed {
    Local(PathBuf),
    Http(String), // base URL, no trailing slash
}

impl Feed {
    fn display(&self) -> String {
        match self {
            Feed::Local(p) => p.display().to_string(),
            Feed::Http(u) => u.clone(),
        }
    }

    /// First non-empty line of the feed's `version.txt`.
    fn read_version(&self) -> Result<String, String> {
        let raw = match self {
            Feed::Local(dir) => std::fs::read_to_string(dir.join("version.txt"))
                .map_err(|e| format!("Update-Feed nicht lesbar ({}): {}", dir.display(), e))?,
            Feed::Http(base) => http_get_string(&format!("{base}/version.txt"))?,
        };
        Ok(raw.lines().next().unwrap_or("").trim().to_string())
    }

    /// Obtain the new binary as a local file ready for `swap_in`. HTTP feeds
    /// download to a temp file in appdata.
    fn fetch_exe(&self) -> Result<PathBuf, String> {
        match self {
            Feed::Local(dir) => ["Smart Explorer.exe", "smart_explorer.exe"]
                .iter()
                .map(|n| dir.join(n))
                .find(|p| p.exists())
                .ok_or_else(|| format!("Keine EXE im Update-Feed {} gefunden", dir.display())),
            Feed::Http(base) => {
                let dest = appdata_dir().join("update_download.exe");
                let _ = std::fs::remove_file(&dest);
                let mut last_err = String::new();
                for name in ["smart_explorer.exe", "Smart%20Explorer.exe"] {
                    match http_download(&format!("{base}/{name}"), &dest) {
                        Ok(()) => return Ok(dest),
                        Err(e) => last_err = e,
                    }
                }
                Err(format!("Download der EXE fehlgeschlagen: {}", last_err))
            }
        }
    }

    fn is_http(&self) -> bool {
        matches!(self, Feed::Http(_))
    }
}

/// Translate the configured source string into a transport.
fn classify_feed(raw: &str) -> Feed {
    let s = raw.trim();
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        Feed::Http(normalize_http_feed(s))
    } else {
        Feed::Local(PathBuf::from(s))
    }
}

/// Accept a bare GitHub repo URL as shorthand for its raw update-feed folder,
/// so a user can paste the repository link as the update location:
///   https://github.com/<owner>/<repo>            → the `main` branch feed
///   https://github.com/<owner>/<repo>/tree/<ref> → that ref's feed
/// Anything else is used verbatim (trailing slash trimmed).
fn normalize_http_feed(url: &str) -> String {
    const FEED_SUBDIR: &str = "release-native/update-feed";
    let trimmed = url.trim().trim_end_matches('/');
    if let Some(rest) = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
    {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() >= 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            let owner = parts[0];
            let repo = parts[1].trim_end_matches(".git");
            let branch = if parts.len() >= 4 && parts[2] == "tree" {
                parts[3]
            } else {
                "main"
            };
            return format!(
                "https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{FEED_SUBDIR}"
            );
        }
    }
    trimmed.to_string()
}

fn http_get_string(url: &str) -> Result<String, String> {
    let resp = ureq::get(url)
        .timeout(HTTP_TIMEOUT)
        .call()
        .map_err(|e| format!("HTTP {}: {}", url, e))?;
    resp.into_string()
        .map_err(|e| format!("HTTP-Antwort {}: {}", url, e))
}

fn http_download(url: &str, dest: &Path) -> Result<(), String> {
    let resp = ureq::get(url)
        .timeout(HTTP_TIMEOUT)
        .call()
        .map_err(|e| format!("HTTP {}: {}", url, e))?;
    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(dest)
        .map_err(|e| format!("Temp-Datei {}: {}", dest.display(), e))?;
    std::io::copy(&mut reader, &mut file).map_err(|e| format!("Download {}: {}", url, e))?;
    Ok(())
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

/// Filename prefix for the renamed-out running binary (`<stem>_old`).
fn old_binary_prefix(cur_exe: &std::path::Path) -> String {
    let stem = cur_exe
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "smart_explorer".into());
    format!("{}_old", stem)
}

/// A **unique** path to rename the running binary to. Using a timestamp instead
/// of a fixed `<stem>_old.exe` is what makes the rename dance robust: a previous
/// `_old` left locked by a still-running process (e.g. a lingering sync daemon)
/// no longer collides, so renaming the running exe can't hit ACCESS_DENIED
/// (os error 5) on an existing, locked destination.
fn new_old_binary_path(cur_exe: &std::path::Path) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    cur_exe.with_file_name(format!("{}_{}.exe", old_binary_prefix(cur_exe), nanos))
}

/// Delete leftovers from previous updates (best effort, with retries since an
/// old process — the prior GUI or a lingering daemon — may still hold one).
/// Sweeps every `<stem>_old*.exe`, including the legacy fixed name.
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
                        if name.starts_with(&prefix) && name.ends_with(".exe")
                            && std::fs::remove_file(e.path()).is_err()
                        {
                            any_left = true; // still locked — try again shortly
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
    // A fresh, unique destination — never an existing (possibly locked) file.
    let old = new_old_binary_path(&cur_exe);

    std::fs::copy(new_exe, &pending).map_err(|e| format!("Kopieren fehlgeschlagen: {}", e))?;
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
    let raw = match update_source_str() {
        Some(s) => s,
        None => {
            return Ok(if manual { Some(UpdateMsg::NoFeed) } else { None });
        }
    };
    let feed = classify_feed(&raw);

    let feed_version = feed.read_version()?;
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

    // Obtain the new binary (downloaded for http feeds), archive the version
    // we're replacing so it can be rolled back to, then swap it in.
    let new_exe = feed.fetch_exe()?;
    archive_binary(current);
    let swap_result = swap_in(&new_exe);
    // A downloaded temp binary is consumed by swap_in (copied to pending);
    // remove it regardless of outcome.
    if feed.is_http() {
        let _ = std::fs::remove_file(&new_exe);
    }
    let cur_exe = swap_result?;

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
    fn github_repo_url_becomes_raw_feed() {
        assert_eq!(
            normalize_http_feed("https://github.com/b1ue-man/smart-explorer"),
            "https://raw.githubusercontent.com/b1ue-man/smart-explorer/main/release-native/update-feed"
        );
        // trailing slash + .git suffix tolerated
        assert_eq!(
            normalize_http_feed("https://github.com/b1ue-man/smart-explorer.git/"),
            "https://raw.githubusercontent.com/b1ue-man/smart-explorer/main/release-native/update-feed"
        );
        // explicit branch via /tree/<ref>
        assert_eq!(
            normalize_http_feed("https://github.com/o/r/tree/dev"),
            "https://raw.githubusercontent.com/o/r/dev/release-native/update-feed"
        );
        // a non-github URL is passed through verbatim (trailing slash trimmed)
        assert_eq!(
            normalize_http_feed("https://example.com/feed/"),
            "https://example.com/feed"
        );
    }

    #[test]
    fn classify_distinguishes_transports() {
        assert!(matches!(classify_feed("https://example.com/f"), Feed::Http(_)));
        assert!(matches!(classify_feed("http://host/f"), Feed::Http(_)));
        assert!(matches!(classify_feed(r"C:\Users\x\feed"), Feed::Local(_)));
        assert!(matches!(classify_feed(r"\\server\share"), Feed::Local(_)));
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
