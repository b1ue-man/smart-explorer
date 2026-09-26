//! `update.*`: the Android APK from the release update feed (`version.txt`,
//! `smart-explorer-android.apk` + `.sha256`), downloaded into
//! `<cache>/update/` and verified before the host hands it to the installer.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{json, Value};

use super::args::canceled;
use crate::mobile::{ApiError, Runtime, TaskCtx};

/// Built-in feed; `updateFeedUrl` of the init configuration replaces it.
const BUILTIN_FEED: &str = include_str!("../../../../../update_source.txt");
pub(super) const APK_NAME: &str = "smart-explorer-android.apk";
const DAMAGED: &str = "Download beschädigt – nicht installiert";

/// One download at a time: every task writes the same `.part`/APK paths.
static DOWNLOADING: AtomicBool = AtomicBool::new(false);

/// Frees the download when its task ends, also by unwinding.
struct DownloadSlot;

impl Drop for DownloadSlot {
    fn drop(&mut self) {
        DOWNLOADING.store(false, Ordering::Release);
    }
}

fn feed(rt: &Runtime) -> String {
    match rt.config().update_feed_url.as_deref().map(str::trim) {
        Some(url) if !url.is_empty() => url.to_string(),
        _ => builtin_feed().to_string(),
    }
}

pub(super) fn builtin_feed() -> &'static str {
    BUILTIN_FEED.lines().next().unwrap_or("").trim()
}

fn update_dir() -> PathBuf {
    crate::support_dirs::host()
        .map(|host| host.cache_dir.clone())
        .unwrap_or_else(crate::support_dirs::temp_dir)
        .join("update")
}

pub(super) fn check(rt: &Runtime) -> Result<Value, ApiError> {
    let current = rt.config().app_version.clone();
    let latest = crate::updater::read_feed_version(&feed(rt))
        .map_err(|error| ApiError::new("network", error))?;
    Ok(json!({
        "current": current,
        "latest": latest,
        "available": crate::updater::is_newer(&latest, &current),
        "notes": Value::Null,
    }))
}

pub(super) fn download(rt: &Runtime) -> Result<Value, ApiError> {
    if DOWNLOADING.swap(true, Ordering::AcqRel) {
        return Err(ApiError::new(
            "busy",
            "Das Update wird bereits geladen – bitte warten.",
        ));
    }
    let slot = DownloadSlot;
    let feed = feed(rt);
    let current = rt.config().app_version.clone();
    let task = rt.spawn_task("update", "Update laden".to_string(), move |ctx| {
        // Held until the task, its `.part` cleanup included, has ended.
        let _slot = slot;
        download_task(ctx, &feed, &current)
    });
    Ok(json!({ "taskId": task }))
}

fn download_task(ctx: &TaskCtx, feed: &str, current: &str) -> Result<Value, ApiError> {
    ctx.message("Prüfe Update-Feed…");
    let version =
        crate::updater::read_feed_version(feed).map_err(|error| ApiError::new("network", error))?;
    if !crate::updater::is_newer(&version, current) {
        return Err(ApiError::new(
            "conflict",
            format!("Keine neuere Version verfügbar ({version})."),
        ));
    }
    let expected = crate::updater::read_feed_sha256(feed, APK_NAME)
        .map_err(|error| ApiError::new("network", error))?;
    let dir = update_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|error| super::args::io_error("Update-Ordner anlegen", error))?;
    remove_old_downloads(&dir);
    let safe_version: String = version
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let dest = dir.join(format!("smart-explorer-android-{safe_version}.apk"));
    ctx.message(&format!("Lade Version {version}…"));
    let cancel = ctx.cancel_flag();
    let downloaded =
        crate::updater::download_feed_file(feed, APK_NAME, &dest, &cancel, &mut |done: u64,
                                                                                 total: Option<
            u64,
        >| {
            ctx.progress(done, total.unwrap_or(0), 0, 0)
        });
    if let Err(error) = downloaded {
        if ctx.cancelled() {
            return Err(canceled("Download abgebrochen"));
        }
        return Err(ApiError::new("network", error));
    }
    ctx.message("Prüfe SHA-256…");
    let actual = crate::updater::file_sha256(&dest).map_err(|error| {
        let _ = std::fs::remove_file(&dest);
        ApiError::new("internal", error)
    })?;
    if !actual.eq_ignore_ascii_case(&expected) {
        let _ = std::fs::remove_file(&dest);
        return Err(ApiError::new("internal", DAMAGED));
    }
    Ok(json!({
        "path": dest.to_string_lossy(),
        "version": version,
    }))
}

/// Only the newest download is kept.
fn remove_old_downloads(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let ours = name.starts_with("smart-explorer-android")
            && (name.ends_with(".apk") || name.ends_with(".apk.part"));
        if ours && entry.file_type().is_ok_and(|kind| kind.is_file()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}
