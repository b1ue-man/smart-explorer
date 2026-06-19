use std::path::{Path, PathBuf};
use std::time::Duration;

use super::config::{appdata_dir, update_source_str};
use super::core::{parse_sha256_file, parse_ver, staged_payload_path, verify_sha256};

const HTTP_TIMEOUT: Duration = Duration::from_secs(25);
const UPDATE_USER_AGENT: &str = "smart-explorer-updater";

pub(super) enum Feed {
    Local(PathBuf),
    Http(String), // base URL, no trailing slash
}

impl Feed {
    pub(super) fn display(&self) -> String {
        match self {
            Feed::Local(p) => p.display().to_string(),
            Feed::Http(u) => u.clone(),
        }
    }

    /// First non-empty line of the feed's `version.txt`.
    pub(super) fn read_version(&self) -> Result<String, String> {
        let raw = match self {
            Feed::Local(dir) => std::fs::read_to_string(dir.join("version.txt"))
                .map_err(|e| format!("Update-Feed nicht lesbar ({}): {}", dir.display(), e))?,
            Feed::Http(base) => http_get_string(&format!("{base}/version.txt"))?,
        };
        Ok(raw.lines().next().unwrap_or("").trim().to_string())
    }

    /// Stage the new app binary as a local file. Local feeds are copied too, so
    /// a detached helper can delete the staging file without touching the feed.
    pub(super) fn fetch_exe(&self, version: &str) -> Result<PathBuf, String> {
        self.fetch_payload(
            &["smart_explorer.exe", "Smart Explorer.exe"],
            &["smart_explorer.exe", "Smart%20Explorer.exe"],
            "smart_explorer.exe.sha256",
            "update_download",
            version,
        )
    }

    pub(super) fn fetch_updater_exe(&self, version: &str) -> Result<PathBuf, String> {
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
pub(super) fn classify_feed(raw: &str) -> Feed {
    let s = raw.trim();
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        Feed::Http(normalize_http_feed(s))
    } else {
        Feed::Local(PathBuf::from(s))
    }
}

/// Accept a bare GitHub repo URL as shorthand for its raw update-feed folder.
pub(super) fn normalize_http_feed(url: &str) -> String {
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

/// Owner/repo from the configured feed, if it's a GitHub feed.
pub(super) fn github_repo(feed_raw: &str) -> Option<(String, String)> {
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

/// `release/v*` branch name -> version (e.g. "release/v0.5.63" -> "0.5.63").
pub(super) fn tag_to_version(name: &str) -> Option<String> {
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

/// List previously-released versions from the GitHub feed's releases.
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

/// Download a specific released version's binary to a temp file ready for
/// `revert_to`/`swap_in`.
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
