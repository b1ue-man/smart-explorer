use std::path::{Path, PathBuf};
use std::time::Duration;

use super::core::{parse_sha256_file, parse_ver, staged_payload_path, verify_sha256};
use super::os;
use super::release_signature::ReleaseManifest;
use super::types::VerifiedPayload;

#[path = "feed_launcher.rs"]
mod launcher;
#[path = "legacy_launcher.rs"]
mod legacy_launcher;
#[path = "feed_release.rs"]
mod release;
pub use legacy_launcher::{
    complete_legacy_launcher, CompletedLegacyLauncher, LegacyLauncherRequest,
};
pub use release::{download_update, download_version, list_remote_versions, RollbackDownload};

const HTTP_TIMEOUT: Duration = Duration::from_secs(25);
const UPDATE_USER_AGENT: &str = "smart-explorer-updater";

pub(super) struct PayloadSpec {
    pub(super) local_names: &'static [&'static str],
    pub(super) http_names: &'static [&'static str],
    pub(super) hash_name: &'static str,
}

fn app_release_asset_name() -> &'static str {
    os::app_payload_spec().local_names[0]
}

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
    /// With a signed `manifest`, only assets it lists are fetched, the sidecar
    /// must name the signed hash and the payload must have the signed size.
    pub(super) fn fetch_exe(
        &self,
        version: &str,
        manifest: Option<&ReleaseManifest>,
    ) -> Result<VerifiedPayload, String> {
        let spec = os::app_payload_spec();
        self.fetch_payload(
            spec.local_names,
            spec.http_names,
            spec.hash_name,
            "update_download",
            version,
            manifest,
        )
    }

    pub(super) fn fetch_updater_exe(
        &self,
        version: &str,
        manifest: Option<&ReleaseManifest>,
    ) -> Result<VerifiedPayload, String> {
        let spec = os::updater_payload_spec();
        self.fetch_payload(
            spec.local_names,
            spec.http_names,
            spec.hash_name,
            "updater_download",
            version,
            manifest,
        )
    }

    pub(super) fn fetch_cli_exe(
        &self,
        version: &str,
        manifest: Option<&ReleaseManifest>,
    ) -> Result<VerifiedPayload, String> {
        let spec = os::cli_payload_spec();
        self.fetch_payload(
            spec.local_names,
            spec.http_names,
            spec.hash_name,
            "cli_download",
            version,
            manifest,
        )
    }

    fn fetch_payload(
        &self,
        local_names: &[&str],
        http_names: &[&str],
        hash_name: &str,
        temp_prefix: &str,
        version: &str,
        manifest: Option<&ReleaseManifest>,
    ) -> Result<VerifiedPayload, String> {
        let hash = self.read_sha256_required(hash_name)?;
        let dest = staged_payload_path(temp_prefix, version, &hash);
        let _ = std::fs::remove_file(&dest);
        match self {
            Feed::Local(dir) => {
                let (source, size) = local_payload(dir, local_names, &hash, manifest)?;
                check_local_size(&source, size)?;
                std::fs::copy(&source, &dest).map_err(|e| {
                    format!(
                        "Update-Datei stagen ({} -> {}): {}",
                        source.display(),
                        dest.display(),
                        e
                    )
                })?;
                verify_sha256(&dest, &hash)?;
                os::mark_staged_executable(&dest)?;
                VerifiedPayload::new(dest, hash)
            }
            Feed::Http(base) => {
                let mut last_err =
                    format!("{hash_name}: keine Datei im signierten Release-Manifest");
                for name in http_names {
                    let Some(size) = signed_size(manifest, name, &hash)? else {
                        continue;
                    };
                    match http_download(&format!("{base}/{name}"), &dest, size) {
                        Ok(()) => {
                            verify_sha256(&dest, &hash)?;
                            os::mark_staged_executable(&dest)?;
                            return VerifiedPayload::new(dest, hash);
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

    fn read_sha256_required(&self, hash_name: &str) -> Result<String, String> {
        self.read_sha256(hash_name)?.ok_or_else(|| {
            format!(
                "Update-Feed {} enthaelt keine erforderliche Pruefsumme {}",
                self.display(),
                hash_name
            )
        })
    }
}

impl Feed {
    /// This feed for the files of `version` (`pin_release_feed`).
    pub(super) fn pinned(&self, version: &str) -> Feed {
        match self {
            Feed::Http(base) => Feed::Http(pin_release_feed(base, version)),
            Feed::Local(dir) => Feed::Local(dir.clone()),
        }
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

/// Suffix of a repository's release feed: GitHub answers
/// `releases/latest/download/<asset>` with a redirect to that asset of the
/// newest published release.
const LATEST_RELEASE: &str = "/releases/latest/download";

/// A GitHub repository link, its `main` branch and the former raw feed of
/// `main` (`update_source.txt` of installs up to 0.5.190) all mean the
/// repository's release feed; another branch keeps its raw feed folder.
pub(super) fn normalize_http_feed(url: &str) -> String {
    const FEED_SUBDIR: &str = "release-native/update-feed";
    let trimmed = url.trim().trim_end_matches('/');
    let release =
        |owner: &str, repo: &str| format!("https://github.com/{owner}/{repo}{LATEST_RELEASE}");
    if let Some(rest) = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
    {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() >= 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            let owner = parts[0];
            let repo = parts[1].trim_end_matches(".git");
            match &parts[2..] {
                [] | ["tree", "main"] => return release(owner, repo),
                ["tree", branch, ..] => {
                    return format!(
                        "https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{FEED_SUBDIR}"
                    )
                }
                _ => {}
            }
        }
    }
    if let Some(rest) = trimmed.strip_prefix("https://raw.githubusercontent.com/") {
        if let [owner, repo, "main", "release-native", "update-feed"] =
            rest.split('/').collect::<Vec<_>>().as_slice()
        {
            return release(owner, repo);
        }
    }
    trimmed.to_string()
}

/// The feed of exactly `version`: the newest release can change between two
/// requests, so every file of one update comes from the tagged release whose
/// `version.txt` was read.
pub(super) fn pin_release_feed(base: &str, version: &str) -> String {
    match base.strip_suffix(LATEST_RELEASE) {
        Some(repo) => format!("{repo}/releases/download/v{version}"),
        None => base.to_string(),
    }
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

/// The body of `url` up to `limit` bytes; `None` for 404.
pub(super) fn http_get_bytes_optional_404(
    url: &str,
    limit: u64,
) -> Result<Option<Vec<u8>>, String> {
    use std::io::Read;
    let resp = match ureq::get(url)
        .set("User-Agent", UPDATE_USER_AGENT)
        .timeout(HTTP_TIMEOUT)
        .call()
    {
        Ok(resp) => resp,
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(e) => return Err(format_http_error(url, e)),
    };
    let mut bytes = Vec::new();
    resp.into_reader()
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("HTTP-Antwort {}: {}", url, e))?;
    Ok(Some(bytes))
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

/// Downloads `url` to `dest`; `signed_size` (from the release manifest) is
/// the exact length, otherwise the announced length or the free space bounds
/// the download (r-lokal-4).
fn http_download(url: &str, dest: &Path, signed_size: Option<u64>) -> Result<(), String> {
    let tmp = dest.with_extension(format!(
        "{}download",
        dest.extension()
            .and_then(|s| s.to_str())
            .map(|s| format!("{s}."))
            .unwrap_or_default()
    ));
    let _ = std::fs::remove_file(&tmp);
    let resp = ureq::get(url)
        .set("User-Agent", UPDATE_USER_AGENT)
        .timeout(HTTP_TIMEOUT)
        .call()
        .map_err(|e| format_http_error(url, e))?;
    // r-lokal-4: exactly the signed size; without manifest never more than
    // announced, nor than the free space minus the app-data reserve when no
    // length is announced.
    let announced = resp
        .header("Content-Length")
        .and_then(|value| value.trim().parse::<u64>().ok());
    let folder = tmp.parent().unwrap_or_else(|| Path::new("."));
    let bound = match signed_size {
        Some(size) => super::download_bound::DownloadBound::exact(size),
        None => super::download_bound::DownloadBound::new(announced, folder),
    };
    let mut reader = resp.into_reader();
    let mut file =
        std::fs::File::create(&tmp).map_err(|e| format!("Temp-Datei {}: {}", tmp.display(), e))?;
    let copied =
        super::download_bound::copy_bounded(&mut reader, &mut file, bound, &mut |_| {}, &|| false);
    if let Err(e) = copied {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Download {}: {}", url, e));
    }
    if let Err(e) = file.sync_all() {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Download sync {}: {}", url, e));
    }
    drop(file);
    if let Err(e) = std::fs::rename(&tmp, dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Download bereitstellen {}: {}", dest.display(), e));
    }
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
    let rest = base
        .strip_prefix("https://github.com/")
        .or_else(|| base.strip_prefix("https://raw.githubusercontent.com/"))?;
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

/// The asset name a release lists for an HTTP path segment.
fn asset_name(http_name: &str) -> String {
    http_name.replace("%20", " ")
}

/// `Ok(None)`: skip `http_name`, the signed release does not list it.
/// `Ok(Some(size))`: fetch it with that exact size (`None` without manifest).
pub(super) fn signed_size(
    manifest: Option<&ReleaseManifest>,
    http_name: &str,
    sidecar_sha256: &str,
) -> Result<Option<Option<u64>>, String> {
    match manifest {
        None => Ok(Some(None)),
        Some(manifest) => Ok(manifest
            .bind(&asset_name(http_name), sidecar_sha256)?
            .map(Some)),
    }
}

/// The first present local payload (that the signed release lists, if any)
/// and its signed size.
pub(super) fn local_payload(
    dir: &Path,
    local_names: &[&str],
    sidecar_sha256: &str,
    manifest: Option<&ReleaseManifest>,
) -> Result<(PathBuf, Option<u64>), String> {
    for name in local_names {
        let path = dir.join(name);
        if !path.exists() {
            continue;
        }
        if let Some(size) = signed_size(manifest, name, sidecar_sha256)? {
            return Ok((path, size));
        }
    }
    Err(match manifest {
        None => format!("Keine Datei im Update-Feed {} gefunden", dir.display()),
        Some(_) => format!(
            "Keine im signierten Release-Manifest genannte Datei im Update-Feed {} gefunden",
            dir.display()
        ),
    })
}

/// A local payload must have its signed size before it is copied.
pub(super) fn check_local_size(source: &Path, size: Option<u64>) -> Result<(), String> {
    let Some(size) = size else {
        return Ok(());
    };
    let actual = std::fs::metadata(source)
        .map_err(|e| format!("Update-Datei {}: {e}", source.display()))?
        .len();
    if actual != size {
        return Err(format!(
            "update_signature_mismatch: {} has {actual} bytes, the signed release manifest names {size}",
            source.display()
        ));
    }
    Ok(())
}
