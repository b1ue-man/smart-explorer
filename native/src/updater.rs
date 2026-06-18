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
//
// NOTE: Since v0.5.77 the normal auto-update path stages the payload and hands
// replacement to the persistent `Smart Explorer Updater.exe` helper. The
// rename-dance path above remains only for legacy/manual flows.

use crossbeam_channel::Sender;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub enum UpdateMsg {
    /// Feed reachable, no newer version. Only sent for manual checks.
    UpToDate { feed_version: String },
    /// No feed configured. Only sent for manual checks.
    NoFeed,
    /// New binary swapped in — relaunch `exe` with --updated and exit.
    Applied { version: String, exe: PathBuf },
    /// In-place swap couldn't replace the running exe (locked); a detached
    /// worker was launched that will replace + relaunch after we exit. The app
    /// should just close — do NOT relaunch (the worker does).
    AppliedViaWorker { version: String },
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

fn updater_error_path() -> PathBuf {
    appdata_dir().join("last_updater_error.txt")
}

pub fn take_updater_error() -> Option<String> {
    let p = updater_error_path();
    let raw = std::fs::read_to_string(&p).ok()?;
    let _ = std::fs::remove_file(&p);
    let msg = raw.trim().to_string();
    if msg.is_empty() {
        None
    } else {
        Some(msg)
    }
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
const UPDATE_USER_AGENT: &str = "smart-explorer-updater";
const INSTALLED_UPDATER_EXE: &str = "Smart Explorer Updater.exe";

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

    /// Stage the new app binary as a local file. Local feeds are copied too, so
    /// a detached helper can delete the staging file without touching the feed.
    fn fetch_exe(&self, version: &str) -> Result<PathBuf, String> {
        self.fetch_payload(
            &["smart_explorer.exe", "Smart Explorer.exe"],
            &["smart_explorer.exe", "Smart%20Explorer.exe"],
            "smart_explorer.exe.sha256",
            "update_download",
            version,
        )
    }

    fn fetch_updater_exe(&self, version: &str) -> Result<PathBuf, String> {
        self.fetch_payload(
            &["smart_explorer_updater.exe", "Smart Explorer Updater.exe"],
            &[
                "smart_explorer_updater.exe",
                "Smart%20Explorer%20Updater.exe",
            ],
            "smart_explorer_updater.exe.sha256",
            "updater_download",
            version,
        )
    }

    fn fetch_payload(
        &self,
        local_names: &[&str],
        http_names: &[&str],
        hash_name: &str,
        temp_prefix: &str,
        version: &str,
    ) -> Result<PathBuf, String> {
        let dest = staged_payload_path(temp_prefix, version);
        let _ = std::fs::remove_file(&dest);
        match self {
            Feed::Local(dir) => {
                let source = local_names
                    .iter()
                    .map(|n| dir.join(n))
                    .find(|p| p.exists())
                    .ok_or_else(|| {
                        format!("Keine Datei im Update-Feed {} gefunden", dir.display())
                    })?;
                std::fs::copy(&source, &dest).map_err(|e| {
                    format!(
                        "Update-Datei stagen ({} -> {}): {}",
                        source.display(),
                        dest.display(),
                        e
                    )
                })?;
                if let Some(hash) = self.read_sha256(hash_name)? {
                    verify_sha256(&dest, &hash)?;
                }
                Ok(dest)
            }
            Feed::Http(base) => {
                let mut last_err = String::new();
                for name in http_names {
                    match http_download(&format!("{base}/{name}"), &dest) {
                        Ok(()) => {
                            if let Some(hash) = self.read_sha256(hash_name)? {
                                verify_sha256(&dest, &hash)?;
                            }
                            return Ok(dest);
                        }
                        Err(e) => last_err = e,
                    }
                }
                Err(format!(
                    "Download der Update-Datei fehlgeschlagen: {}",
                    last_err
                ))
            }
        }
    }

    fn read_sha256(&self, hash_name: &str) -> Result<Option<String>, String> {
        let raw = match self {
            Feed::Local(dir) => {
                let p = dir.join(hash_name);
                match std::fs::read_to_string(&p) {
                    Ok(s) => Some(s),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => return Err(format!("Hash-Datei {}: {}", p.display(), e)),
                }
            }
            Feed::Http(base) => http_get_string_optional_404(&format!("{base}/{hash_name}"))?,
        };
        raw.map(|s| parse_sha256_file(&s, hash_name)).transpose()
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
        .set("User-Agent", UPDATE_USER_AGENT)
        .timeout(HTTP_TIMEOUT)
        .call()
        .map_err(|e| format_http_error(url, e))?;
    resp.into_string()
        .map_err(|e| format!("HTTP-Antwort {}: {}", url, e))
}

fn http_get_string_optional_404(url: &str) -> Result<Option<String>, String> {
    let resp = match ureq::get(url)
        .set("User-Agent", UPDATE_USER_AGENT)
        .timeout(HTTP_TIMEOUT)
        .call()
    {
        Ok(resp) => resp,
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(e) => return Err(format_http_error(url, e)),
    };
    resp.into_string()
        .map(Some)
        .map_err(|e| format!("HTTP-Antwort {}: {}", url, e))
}

fn http_download(url: &str, dest: &Path) -> Result<(), String> {
    let resp = ureq::get(url)
        .set("User-Agent", UPDATE_USER_AGENT)
        .timeout(HTTP_TIMEOUT)
        .call()
        .map_err(|e| format_http_error(url, e))?;
    let mut reader = resp.into_reader();
    let mut file =
        std::fs::File::create(dest).map_err(|e| format!("Temp-Datei {}: {}", dest.display(), e))?;
    std::io::copy(&mut reader, &mut file).map_err(|e| format!("Download {}: {}", url, e))?;
    Ok(())
}

fn format_http_error(url: &str, err: ureq::Error) -> String {
    let msg = err.to_string();
    let hint = if msg.contains("os error 10013")
        || msg.contains("Zugriff auf einen Socket")
        || msg.contains("access permissions")
    {
        " Hinweis: Windows hat den ausgehenden Socket blockiert. Pruefe Firewall/Antivirus oder eine App-Regel fuer Smart Explorer."
    } else {
        ""
    };
    format!("HTTP {}: {}{}", url, msg, hint)
}

fn staged_payload_path(prefix: &str, version: &str) -> PathBuf {
    let safe_version: String = version
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    appdata_dir().join(format!(
        "{}_{}_{}_{}.exe",
        prefix,
        safe_version,
        std::process::id(),
        nanos
    ))
}

fn parse_sha256_file(raw: &str, name: &str) -> Result<String, String> {
    let token = raw
        .split_whitespace()
        .next()
        .ok_or_else(|| format!("Hash-Datei {} ist leer", name))?
        .to_ascii_lowercase();
    if token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(token)
    } else {
        Err(format!(
            "Hash-Datei {} enthaelt keinen gueltigen SHA-256",
            name
        ))
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("Hash lesen {}: {}", path.display(), e))?;
    let mut hasher = <sha2::Sha256 as sha2::Digest>::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("Hash lesen {}: {}", path.display(), e))?;
        if n == 0 {
            break;
        }
        sha2::Digest::update(&mut hasher, &buf[..n]);
    }
    Ok(format!("{:x}", sha2::Digest::finalize(hasher)))
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let actual = sha256_file(path)?;
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        let _ = std::fs::remove_file(path);
        Err(format!(
            "Download-Pruefsumme passt nicht fuer {}: erwartet {}, erhalten {}",
            path.display(),
            expected,
            actual
        ))
    }
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
                        if name.starts_with(&prefix)
                            && name.ends_with(".exe")
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

/// Owner/repo from the configured feed, if it's a GitHub feed (so we can list
/// and fetch *released* versions from the repo's `release/v*` branches instead
/// of only what's archived locally).
fn github_repo(feed_raw: &str) -> Option<(String, String)> {
    let base = normalize_http_feed(feed_raw);
    let rest = base.strip_prefix("https://raw.githubusercontent.com/")?;
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.len() >= 2 && !parts[0].is_empty() && !parts[1].is_empty() {
        Some((
            parts[0].to_string(),
            parts[1].trim_end_matches(".git").to_string(),
        ))
    } else {
        None
    }
}

/// `release/v*` branch name → version (e.g. "release/v0.5.63" → "0.5.63").
fn tag_to_version(name: &str) -> Option<String> {
    let v = name.strip_prefix('v')?;
    if v.chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false)
    {
        Some(v.to_string())
    } else {
        None
    }
}

/// List previously-RELEASED versions from the GitHub feed's `release/v*`
/// branches (newest first, current excluded). Empty for non-GitHub feeds or on
/// any network error — callers fall back to the locally-archived list. Network;
/// run off the UI thread.
pub fn list_remote_versions() -> Vec<String> {
    let raw = match update_source_str() {
        Some(s) => s,
        None => return Vec::new(),
    };
    let (owner, repo) = match github_repo(&raw) {
        Some(v) => v,
        None => return Vec::new(),
    };
    let cur = env!("CARGO_PKG_VERSION");
    let mut versions: Vec<String> = Vec::new();
    // GitHub paginates releases at 100; a few pages cover the project's history.
    for page in 1..=5u32 {
        let url = format!(
            "https://api.github.com/repos/{owner}/{repo}/releases?per_page=100&page={page}"
        );
        let body = match http_get_github_json(&url) {
            Ok(s) => s,
            Err(_) => break,
        };
        let arr: Vec<serde_json::Value> = match serde_json::from_str(&body) {
            Ok(a) => a,
            Err(_) => break,
        };
        let n = arr.len();
        for release in &arr {
            if release.get("draft").and_then(|v| v.as_bool()) == Some(true) {
                continue;
            }
            let has_app_asset = release
                .get("assets")
                .and_then(|v| v.as_array())
                .map(|assets| {
                    assets.iter().any(|asset| {
                        asset.get("name").and_then(|v| v.as_str()) == Some("smart_explorer.exe")
                    })
                })
                .unwrap_or(true);
            if !has_app_asset {
                continue;
            }
            if let Some(v) = release
                .get("tag_name")
                .and_then(|v| v.as_str())
                .and_then(tag_to_version)
            {
                versions.push(v);
            }
        }
        if n < 100 {
            break;
        }
    }
    versions.retain(|v| v != cur);
    versions.sort_by(|a, b| parse_ver(b).cmp(&parse_ver(a)));
    versions.dedup();
    versions
}

fn http_get_github_json(url: &str) -> Result<String, String> {
    let resp = ureq::get(url)
        .set("User-Agent", UPDATE_USER_AGENT)
        .set("Accept", "application/vnd.github+json")
        .timeout(HTTP_TIMEOUT)
        .call()
        .map_err(|e| format_http_error(url, e))?;
    resp.into_string()
        .map_err(|e| format!("HTTP-Antwort {}: {}", url, e))
}

/// Download a specific released version's binary (from its `release/v<ver>`
/// branch on the GitHub feed) to a temp file ready for `revert_to`/`swap_in`.
pub fn download_version(version: &str) -> Result<PathBuf, String> {
    let raw = update_source_str().ok_or("Keine Update-Quelle konfiguriert")?;
    let (owner, repo) =
        github_repo(&raw).ok_or("Frühere Versionen sind nur über einen GitHub-Feed abrufbar")?;
    let url =
        format!("https://github.com/{owner}/{repo}/releases/download/v{version}/smart_explorer.exe");
    let dest = appdata_dir().join("rollback_download.exe");
    let _ = std::fs::remove_file(&dest);
    if let Err(release_err) = http_download(&url, &dest) {
        let branch_url = format!(
            "https://raw.githubusercontent.com/{owner}/{repo}/release/v{version}/release-native/update-feed/smart_explorer.exe"
        );
        http_download(&branch_url, &dest).map_err(|branch_err| {
            format!(
                "Release-Download fehlgeschlagen: {}; Branch-Fallback fehlgeschlagen: {}",
                release_err, branch_err
            )
        })?;
    }
    Ok(dest)
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
        format!(
            "Programmdatei kann nicht ersetzt werden ({}): {}",
            cur_exe.display(),
            e
        )
    })?;
    if let Err(e) = std::fs::rename(&pending, &cur_exe) {
        let _ = std::fs::rename(&old, &cur_exe);
        let _ = std::fs::remove_file(&pending);
        return Err(format!("Einsetzen fehlgeschlagen: {}", e));
    }
    Ok(cur_exe)
}

/// Spawn a process fully detached: no console window, and (on Windows) broken
/// away from any job object so it outlives this process. Used for the update
/// worker and relaunch.
fn spawn_detached(exe: &std::path::Path, args: &[&str]) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB);
        match cmd.spawn() {
            Ok(_) => return Ok(()),
            Err(e) if should_elevate_for_spawn(&e) => {
                return spawn_elevated_detached(exe, args);
            }
            Err(_) => {}
        }
        // Some job objects forbid breakaway → retry without that flag.
        let mut c2 = std::process::Command::new(exe);
        c2.args(args)
            .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
        return match c2.spawn() {
            Ok(_) => Ok(()),
            Err(e) if should_elevate_for_spawn(&e) => spawn_elevated_detached(exe, args),
            Err(e) => Err(e),
        };
    }
    #[cfg(not(windows))]
    {
        cmd.spawn().map(|_| ())
    }
}

#[cfg(windows)]
fn should_elevate_for_spawn(e: &std::io::Error) -> bool {
    matches!(e.raw_os_error(), Some(5) | Some(740) | Some(1314))
        || e.kind() == std::io::ErrorKind::PermissionDenied
}

#[cfg(windows)]
fn spawn_elevated_detached(exe: &std::path::Path, args: &[&str]) -> std::io::Result<()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide_os(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }
    fn wide_str(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let verb = wide_str("runas");
    let file = wide_os(exe.as_os_str());
    let params = wide_str(&join_windows_args(args));
    let rc = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if rc > 32 {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("Administratorfreigabe abgebrochen oder verweigert (ShellExecuteW={rc})"),
        ))
    }
}

#[cfg(windows)]
fn join_windows_args(args: &[&str]) -> String {
    args.iter()
        .map(|arg| quote_windows_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(windows)]
fn quote_windows_arg(arg: &str) -> String {
    if !arg.is_empty()
        && !arg
            .chars()
            .any(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '"'))
    {
        return arg.to_string();
    }

    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                out.push(ch);
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

fn installed_updater_path() -> Result<PathBuf, String> {
    let cur = std::env::current_exe().map_err(|e| format!("Eigener Pfad unbekannt: {}", e))?;
    let dir = cur
        .parent()
        .ok_or_else(|| format!("Installationsordner unbekannt: {}", cur.display()))?;
    Ok(dir.join(INSTALLED_UPDATER_EXE))
}

fn copy_with_retries(src: &Path, dest: &Path, label: &str) -> Result<(), String> {
    let mut last = None;
    for _ in 0..10 {
        match std::fs::copy(src, dest) {
            Ok(_) => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(350));
            }
        }
    }
    Err(format!(
        "{} kopieren ({} -> {}): {}",
        label,
        src.display(),
        dest.display(),
        last.map(|e| e.to_string())
            .unwrap_or_else(|| "unbekannter Fehler".to_string())
    ))
}

fn ensure_installed_updater(feed: &Feed, version: &str, refresh: bool) -> Result<PathBuf, String> {
    let dest = installed_updater_path()?;
    if !refresh && dest.exists() {
        return Ok(dest);
    }

    match feed.fetch_updater_exe(version) {
        Ok(staged) => {
            let result = copy_with_retries(&staged, &dest, "Updater-Helfer");
            let _ = std::fs::remove_file(&staged);
            result?;
            Ok(dest)
        }
        Err(_e) if dest.exists() => Ok(dest),
        Err(e) => Err(format!(
            "Updater-Helfer fehlt und konnte nicht aus dem Feed geladen werden: {}",
            e
        )),
    }
}

fn apply_via_installed_updater(
    helper: &Path,
    staged_exe: &Path,
    version: &str,
) -> Result<(), String> {
    let cur_exe = std::env::current_exe().map_err(|e| format!("Eigener Pfad unbekannt: {}", e))?;
    let target = cur_exe.to_string_lossy().into_owned();
    let staged = staged_exe.to_string_lossy().into_owned();
    let parent_pid = std::process::id().to_string();
    let last_applied = last_applied_path().to_string_lossy().into_owned();
    let error_file = updater_error_path().to_string_lossy().into_owned();
    spawn_detached(
        helper,
        &[
            "--apply",
            "--target",
            &target,
            "--staged",
            &staged,
            "--parent-pid",
            &parent_pid,
            "--version",
            version,
            "--last-applied",
            &last_applied,
            "--error-file",
            &error_file,
        ],
    )
    .map_err(|e| format!("Updater-Helfer starten: {}", e))?;
    Ok(())
}

/// Fallback when the in-place swap can't replace the running exe: stage the new
/// binary to a temp location and launch IT as a detached worker that waits for
/// us (our PID) to exit, then copies itself over the target and relaunches it.
/// The worker never runs from the file it replaces, so locks can't block it.
fn apply_via_worker(new_exe: &std::path::Path) -> Result<(), String> {
    let cur_exe = std::env::current_exe().map_err(|e| format!("Eigener Pfad unbekannt: {}", e))?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let worker = std::env::temp_dir().join(format!("se_update_worker_{}.exe", nanos));
    std::fs::copy(new_exe, &worker).map_err(|e| format!("Worker stagen: {}", e))?;
    let pid = std::process::id().to_string();
    spawn_detached(
        &worker,
        &["--apply-update", &cur_exe.to_string_lossy(), &pid],
    )
    .map_err(|e| format!("Worker starten: {}", e))?;
    Ok(())
}

/// Worker entry point (`--apply-update <target> <parent_pid>`). Runs from a temp
/// copy of the NEW binary; waits for the parent to exit, replaces `target` with
/// itself, relaunches it, and exits. Best-effort — on failure the target keeps
/// the old (working) binary, so a botched update can never brick the app.
pub fn run_apply_worker(args: &[String]) {
    let i = match args.iter().position(|a| a == "--apply-update") {
        Some(i) => i,
        None => return,
    };
    let target = match args.get(i + 1) {
        Some(t) => PathBuf::from(t),
        None => return,
    };
    let parent_pid: u32 = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let src = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };

    wait_for_pid_exit(parent_pid, Duration::from_secs(30));

    // Replace the target, retrying while it may still be briefly locked.
    let mut replaced = false;
    for _ in 0..60 {
        if std::fs::copy(&src, &target).is_ok() {
            replaced = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    if replaced {
        let _ = spawn_detached(&target, &["--updated"]);
    }
}

/// Wait until process `pid` has exited, or `timeout` elapses. On Windows this
/// waits on the process handle; elsewhere it polls. pid 0 = skip.
fn wait_for_pid_exit(pid: u32, timeout: Duration) {
    if pid == 0 {
        return;
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
        };
        unsafe {
            let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if !h.is_null() {
                WaitForSingleObject(h, timeout.as_millis() as u32);
                CloseHandle(h);
                return;
            }
        }
        // OpenProcess failed (already gone / no rights) → small settle delay.
        std::thread::sleep(Duration::from_millis(300));
    }
    #[cfg(not(windows))]
    {
        let _ = timeout;
        std::thread::sleep(Duration::from_millis(300));
    }
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

/// True if released `candidate` is strictly newer than `current` (semver-ish).
/// Lets the UI tell apart "newer release → offer as an update" from "older
/// release → offer as a rollback".
pub fn is_newer(candidate: &str, current: &str) -> bool {
    parse_ver(candidate) > parse_ver(current)
}

/// Install a downloaded released binary as a FORWARD update: archive the current
/// exe (so the user can still roll back), swap the new one in, and clear any
/// rollback pin so auto-update keeps working. Mirrors `revert_to` but for going
/// forward to a newer release (no pin). Returns the exe to relaunch.
pub fn install_version(downloaded: &std::path::Path, version: &str) -> Result<PathBuf, String> {
    if !downloaded.exists() {
        return Err("Heruntergeladene Version nicht gefunden".into());
    }
    archive_binary(env!("CARGO_PKG_VERSION"));
    if let Ok(helper) = installed_updater_path() {
        if helper.exists() {
            resume_auto_update();
            apply_via_installed_updater(&helper, downloaded, version)?;
            return Ok(PathBuf::new());
        }
    }
    let cur_exe = swap_in(downloaded)?;
    resume_auto_update(); // forward update → don't leave a rollback pin behind
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
            return Ok(if manual {
                Some(UpdateMsg::NoFeed)
            } else {
                None
            });
        }
    };
    let feed = classify_feed(&raw);

    let feed_version = feed.read_version()?;
    if feed_version.is_empty() {
        return Err(format!("version.txt im Feed {} ist leer", feed.display()));
    }

    let current = env!("CARGO_PKG_VERSION");
    if parse_ver(&feed_version) <= parse_ver(current) {
        if parse_ver(&feed_version) == parse_ver(current) {
            let _ = ensure_installed_updater(&feed, &feed_version, false);
        }
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

    // Stage the new binary, archive the outgoing version, refresh the helper,
    // and let the helper replace the app after this process exits.
    let new_exe = feed.fetch_exe(&feed_version)?;
    archive_binary(current);
    let helper = ensure_installed_updater(&feed, &feed_version, true)?;
    apply_via_installed_updater(&helper, &new_exe, &feed_version)?;
    resume_auto_update();
    Ok(Some(UpdateMsg::AppliedViaWorker {
        version: feed_version.clone(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_newer_compares_semver() {
        assert!(is_newer("0.5.74", "0.5.73"));
        assert!(is_newer("0.6.0", "0.5.99"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.5.73", "0.5.73"));
        assert!(!is_newer("0.5.72", "0.5.73"));
        assert!(!is_newer("0.5.9", "0.5.10")); // numeric, not lexical
    }

    #[test]
    fn github_repo_and_release_tag_parsing() {
        // Bare repo URL → owner/repo.
        assert_eq!(
            github_repo("https://github.com/b1ue-man/smart-explorer"),
            Some(("b1ue-man".into(), "smart-explorer".into()))
        );
        // Already-raw feed URL → owner/repo.
        assert_eq!(
            github_repo("https://raw.githubusercontent.com/o/r/main/release-native/update-feed"),
            Some(("o".into(), "r".into()))
        );
        // Non-GitHub feed → None (callers fall back to local archives).
        assert_eq!(github_repo("https://example.com/feed"), None);
        assert_eq!(github_repo("/local/dir"), None);
        // Branch → version.
        assert_eq!(tag_to_version("v0.5.63"), Some("0.5.63".into()));
        assert_eq!(tag_to_version("vX"), None);
        assert_eq!(tag_to_version("main"), None);
        assert_eq!(tag_to_version("release/v0.5.63"), None);
    }

    #[test]
    fn archived_versions_parse_and_sort_numerically() {
        let vd = versions_dir().expect("versions dir");
        std::fs::create_dir_all(&vd).unwrap();
        let mk = ["0.3.6", "0.3.10", "0.4.0"];
        for v in mk {
            std::fs::write(vd.join(format!("Smart Explorer {}.exe", v)), b"x").unwrap();
        }
        let vers: Vec<String> = list_archived_versions()
            .into_iter()
            .map(|(v, _)| v)
            .collect();
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
        assert!(matches!(
            classify_feed("https://example.com/f"),
            Feed::Http(_)
        ));
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
