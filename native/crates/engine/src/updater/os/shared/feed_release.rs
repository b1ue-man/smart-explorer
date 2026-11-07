//! Downloads of one GitHub release: the release list, the rollback binary
//! and the staging of a chosen release, each behind the release signature
//! (E12).
use std::path::PathBuf;

use super::{
    app_release_asset_name, format_http_error, github_repo, http_download, http_get_string, os,
    parse_sha256_file, parse_ver, signed_size, staged_payload_path, tag_to_version, verify_sha256,
    Feed, ReleaseManifest, HTTP_TIMEOUT, UPDATE_USER_AGENT,
};
use crate::updater::config::update_source_str;
use crate::updater::feed_trust::{read_trust, UnsignedConsent, UnsignedFeed};
use crate::updater::staging::stage_with_trust;
use crate::updater::types::UpdateMsg;

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
                        asset.get("name").and_then(|v| v.as_str()) == Some(app_release_asset_name())
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
    versions.sort_by_key(|version| std::cmp::Reverse(parse_ver(version)));
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

/// The rollback download of one released app binary.
#[derive(Debug)]
pub enum RollbackDownload {
    /// The verified binary, ready for `revert_to`.
    Ready(PathBuf),
    /// The release is unsigned; download again with the user's consent.
    NeedsConsent(UnsignedFeed),
}

/// Download a specific released version's binary to a temp file ready for
/// `revert_to`/`swap_in`. An unsigned release of the project's feed needs
/// `consent` for exactly `version` (E12).
pub fn download_version(
    version: &str,
    consent: &UnsignedConsent,
) -> Result<RollbackDownload, String> {
    let raw = update_source_str().ok_or("Keine Update-Quelle konfiguriert")?;
    let (owner, repo) =
        github_repo(&raw).ok_or("Frühere Versionen sind nur über einen GitHub-Feed abrufbar")?;
    // Every published version keeps its assets in its GitHub release; the
    // former `release/v*` branch feeds no longer carry binaries.
    let base = format!("https://github.com/{owner}/{repo}/releases/download/v{version}");
    let trust = read_trust(&Feed::Http(base.clone()), version)?;
    if let Some(kind) = trust.missing_consent(version, consent) {
        return Ok(RollbackDownload::NeedsConsent(kind));
    }
    let spec = os::app_payload_spec();
    let url = format!("{base}/{}", app_release_asset_name());
    let hash_url = format!("{base}/{}", spec.hash_name);
    download_with_required_sha256(
        &url,
        &hash_url,
        spec.hash_name,
        "rollback_download",
        version,
        trust.manifest(),
    )
    .map(RollbackDownload::Ready)
}

/// Download and durably stage all payloads for a specific GitHub release.
/// This function never changes installed files or launches the updater.
/// Answers `UpdateMsg::Staged`, or `UpdateMsg::UnsignedConsentNeeded` for an
/// unsigned release without `consent` for `version` (E12).
pub fn download_update(version: &str, consent: &UnsignedConsent) -> Result<UpdateMsg, String> {
    let raw = update_source_str().ok_or("Keine Update-Quelle konfiguriert")?;
    let (owner, repo) =
        github_repo(&raw).ok_or("Versionen sind nur ueber einen GitHub-Feed abrufbar")?;
    let release = Feed::Http(format!(
        "https://github.com/{owner}/{repo}/releases/download/v{version}"
    ));
    let trust = read_trust(&release, version)?;
    if let Some(feed) = trust.missing_consent(version, consent) {
        return Ok(UpdateMsg::UnsignedConsentNeeded {
            version: version.to_string(),
            feed,
        });
    }
    stage_with_trust(&release, version, &trust).map(UpdateMsg::Staged)
}

fn download_with_required_sha256(
    url: &str,
    hash_url: &str,
    hash_name: &str,
    temp_prefix: &str,
    version: &str,
    manifest: Option<&ReleaseManifest>,
) -> Result<PathBuf, String> {
    let raw = match http_get_string(hash_url) {
        Ok(raw) => raw,
        Err(e) => {
            return Err(format!("Pruefsumme laden: {}", e));
        }
    };
    let hash = parse_sha256_file(&raw, hash_name)?;
    let size = signed_size(manifest, app_release_asset_name(), &hash)?.ok_or_else(|| {
        format!(
            "update_signature_mismatch: the signed release manifest of v{version} does not list {}",
            app_release_asset_name()
        )
    })?;
    let dest = staged_payload_path(temp_prefix, version, &hash);
    let _ = std::fs::remove_file(&dest);
    http_download(url, &dest, size)?;
    verify_sha256(&dest, &hash)?;
    Ok(dest)
}
