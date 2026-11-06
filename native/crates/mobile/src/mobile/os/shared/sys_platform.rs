//! Own Android platform figures and partial MediaStore hints. No root I/O.
use crate::analytics::PlatformTotals;
use crate::mobile::{ApiError, Runtime};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
static NEXT_PROBE: AtomicU64 = AtomicU64::new(1);

/// The cursor last handed to the daemon per volume root: the host's own
/// cursor (`raw`) and the `partial:<nonce>:<raw>` form issued for it.
static ISSUED: Mutex<Vec<(PathBuf, String, String)>> = Mutex::new(Vec::new());

fn issued() -> MutexGuard<'static, Vec<(PathBuf, String, String)>> {
    // Every update leaves the list consistent; a panic elsewhere must not
    // take the cursors down with it.
    ISSUED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The cursor the daemon sees for `root`. MediaStore is partial, so the host
/// cursor never certifies an unchanged file system and always carries a
/// probe nonce; but the nonce only moves when the host reports a change or
/// its cursor moved. A probe that finds nothing new hands over the cursor of
/// the last one, so the daemon can tell an idle probe from a change instead
/// of seeing a new value every minute. A missing cursor stays unknown and
/// forgets the remembered one.
fn issue_cursor(root: &Path, raw: Option<&str>, changed: bool) -> Option<String> {
    let mut issued = issued();
    let Some(raw) = raw else {
        issued.retain(|(known, _, _)| known != root);
        return None;
    };
    if let Some((_, known_raw, cursor)) = issued.iter().find(|(known, _, _)| known == root) {
        if !changed && known_raw == raw {
            return Some(cursor.clone());
        }
    }
    let cursor = format!(
        "partial:{}:{raw}",
        NEXT_PROBE.fetch_add(1, Ordering::Relaxed)
    );
    issued.retain(|(known, _, _)| known != root);
    issued.push((root.to_path_buf(), raw.to_string(), cursor.clone()));
    Some(cursor)
}

/// Shared with analyze.start: unknown numbers stay unknown; cache is clamped
/// by PlatformTotals::add_app as before.
pub(crate) fn platform_totals(args: &Value) -> PlatformTotals {
    let platform = args.get("platform");
    let total = |key: &str| platform?.get(key)?.as_u64();
    let mut totals = PlatformTotals {
        volume_used_bytes: total("volumeUsedBytes"),
        other_apps_bytes: total("otherAppsBytes"),
        apps: Vec::new(),
    };
    let apps = platform
        .and_then(|platform| platform.get("apps"))
        .and_then(Value::as_array);
    for app in apps.into_iter().flatten() {
        let text = |key: &str| app.get(key).and_then(Value::as_str).map(str::trim);
        let Some(package) = text("package").filter(|package| !package.is_empty()) else {
            continue;
        };
        let bytes = |key: &str| app.get(key).and_then(Value::as_u64).unwrap_or(0);
        totals.add_app(
            package.to_string(),
            text("label").unwrap_or_default().to_string(),
            bytes("appBytes"),
            bytes("dataBytes"),
            bytes("cacheBytes"),
        );
    }
    totals
}

pub(super) fn remember(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let volume = super::super::args::str_arg(args, "volume")?;
    let path = Path::new(volume);
    if !path.is_absolute()
        || !rt
            .volumes()
            .iter()
            .any(|known| known.primary && Path::new(&known.path) == path)
    {
        return Err(ApiError::invalid(
            "Plattformzahlen benötigen das tatsächlich primäre lokale Volume.",
        ));
    }
    if !args.get("platform").is_some_and(Value::is_object) {
        return Err(ApiError::invalid("Die erhobenen Plattformzahlen fehlen."));
    }
    crate::analytics::remember_platform_totals(path, &platform_totals(args));
    Ok(json!({ "remembered": true }))
}

pub(super) fn hints(rt: &Runtime, args: &Value) -> Result<Value, ApiError> {
    let hints = args
        .get("volumes")
        .and_then(Value::as_array)
        .filter(|hints| hints.len() <= 128)
        .ok_or_else(|| ApiError::invalid("Ungültige Volume-Hinweise."))?;
    let known = rt.volumes();
    let mut changed = Vec::new();
    for hint in hints {
        let path = super::super::args::str_arg(hint, "path")?;
        let root = Path::new(path);
        if !root.is_absolute() || !known.iter().any(|volume| Path::new(&volume.path) == root) {
            return Err(ApiError::invalid(
                "Der MediaStore-Hinweis gehört zu keinem gemeldeten Volume.",
            ));
        }
        let cursor = hint
            .get("cursor")
            .and_then(Value::as_str)
            .filter(|cursor| cursor.len() <= 2048);
        let reported = super::super::args::bool_or(hint, "changed", true);
        // MediaStore is partial, including when generation is unchanged.
        // Never let its cursor certify an unchanged filesystem to the daemon.
        crate::watch::set_host_cursor(root, issue_cursor(root, cursor, reported));
        if reported {
            changed.push(root.to_path_buf());
        }
    }
    // Volumes that are gone no longer keep a remembered cursor.
    issued().retain(|(root, _, _)| known.iter().any(|volume| Path::new(&volume.path) == root));
    crate::watch::report_host_change(&changed);
    Ok(json!({ "accepted": hints.len(), "complete": false }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_task_idle_probe_keeps_the_cursor_and_a_change_moves_it() {
        let root = Path::new("/battery-task/volume");
        let first = issue_cursor(root, Some("vol:1:10"), true).expect("cursor");
        assert!(first.starts_with("partial:") && first.ends_with(":vol:1:10"));
        // Nothing new: same cursor, so the daemon sees no change.
        assert_eq!(
            issue_cursor(root, Some("vol:1:10"), false),
            Some(first.clone())
        );
        // The host reports a change (observer signal) with the same generation.
        let signalled = issue_cursor(root, Some("vol:1:10"), true).expect("cursor");
        assert_ne!(signalled, first);
        // The generation moved.
        let moved = issue_cursor(root, Some("vol:1:11"), false).expect("cursor");
        assert_ne!(moved, signalled);
        assert!(moved.ends_with(":vol:1:11"));
        // Unknown stays unknown and forgets; the next known cursor is new.
        assert_eq!(issue_cursor(root, None, true), None);
        let again = issue_cursor(root, Some("vol:1:11"), false).expect("cursor");
        assert_ne!(again, moved);
        issued().retain(|(known, _, _)| known != root);
    }

    #[test]
    fn android_host_platform_unknown_totals_are_not_zero() {
        let totals =
            platform_totals(&json!({"platform":{"volumeUsedBytes":null,"otherAppsBytes":-1}}));
        assert_eq!(totals.volume_used_bytes, None);
        assert_eq!(totals.other_apps_bytes, None);
        assert!(totals.apps.is_empty());
    }
}
