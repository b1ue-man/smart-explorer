//! Own Android platform figures and partial MediaStore hints. No root I/O.
use crate::analytics::PlatformTotals;
use crate::mobile::{ApiError, Runtime};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT_PROBE: AtomicU64 = AtomicU64::new(1);

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
        // MediaStore is partial, including when generation is unchanged.
        // Never let its cursor certify an unchanged filesystem to the daemon.
        let cursor = cursor.map(|cursor| {
            format!(
                "partial:{}:{cursor}",
                NEXT_PROBE.fetch_add(1, Ordering::Relaxed)
            )
        });
        crate::watch::set_host_cursor(root, cursor);
        if super::super::args::bool_or(hint, "changed", true) {
            changed.push(root.to_path_buf());
        }
    }
    crate::watch::report_host_change(&changed);
    Ok(json!({ "accepted": hints.len(), "complete": false }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn android_host_platform_unknown_totals_are_not_zero() {
        let totals =
            platform_totals(&json!({"platform":{"volumeUsedBytes":null,"otherAppsBytes":-1}}));
        assert_eq!(totals.volume_used_bytes, None);
        assert_eq!(totals.other_apps_bytes, None);
        assert!(totals.apps.is_empty());
    }
}
