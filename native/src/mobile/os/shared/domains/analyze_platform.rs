//! Android's figures of `analyze.start`: where a local root lies on its
//! volume and the `platform` totals Kotlin measured for that volume (used
//! space, other apps' data, the installed apps).
use std::path::Path;

use serde_json::Value;

use crate::analytics::{PlatformTotals, VolumeRoot};
use crate::mobile::Runtime;

/// Where a local root lies on its volume; Android reports other apps' data
/// and the installed apps only for the primary one.
pub(super) fn volume_root(rt: &Runtime, location: &str) -> VolumeRoot {
    let Some(place) = crate::apptrash::volume_place(Path::new(location)) else {
        return VolumeRoot::default();
    };
    let primary = rt
        .volumes()
        .iter()
        .any(|volume| volume.primary && Path::new(&volume.path) == place.volume.as_path());
    VolumeRoot::from_segments(&place.below, primary)
}

pub(super) fn remember_totals(rt: &Runtime, location: &str, totals: &PlatformTotals) {
    if let Some(place) = crate::apptrash::volume_place(Path::new(location)) {
        if rt.volumes().iter().any(|volume| volume.primary && Path::new(&volume.path) == place.volume.as_path()) {
            crate::analytics::remember_platform_totals(&place.volume, totals);
        }
    }
}

/// `platform: {volumeUsedBytes?, otherAppsBytes?, apps?: [{package, label,
/// appBytes, dataBytes, cacheBytes}]}`: missing, `null` or negative totals
/// are unknown, entries without a package are skipped and missing or
/// negative app figures count as 0.
pub(super) fn platform_totals(args: &Value) -> PlatformTotals {
    super::super::super::sys::platform::platform_totals(args)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::{bind_task, node, Pending, Stored};
    use super::platform_totals;
    use crate::analytics::{Approximations, ScanOutcome, SizeNode, VolumeRoot};

    #[test]
    fn android_background_task_platform_reads_totals_and_apps() {
        let args = json!({ "location": "/x", "platform": {
            "volumeUsedBytes": 5000,
            "otherAppsBytes": -1,
            "apps": [
                { "package": "com.game", "label": " Game ", "appBytes": 1400, "dataBytes": 300, "cacheBytes": 900 },
                { "package": " ", "label": "Ohne Paket", "appBytes": 1 },
                { "label": "Auch ohne" },
                { "package": "com.bare", "appBytes": -5, "dataBytes": null },
                "kein Objekt"
            ]
        }});
        let totals = platform_totals(&args);
        assert_eq!(totals.volume_used_bytes, Some(5000));
        assert_eq!(totals.other_apps_bytes, None);
        let apps: Vec<(&str, &str, (u64, u64, u64))> = totals
            .apps
            .iter()
            .map(|app| {
                let bytes = (app.app_bytes, app.data_bytes, app.cache_bytes);
                (app.package.as_str(), app.label.as_str(), bytes)
            })
            .collect();
        assert_eq!(
            apps,
            [
                ("com.game", "Game", (1400, 300, 300)),
                ("com.bare", "com.bare", (0, 0, 0))
            ]
        );
        for args in [
            json!({}),
            json!({ "platform": null }),
            json!({ "platform": { "apps": {} } }),
        ] {
            let totals = platform_totals(&args);
            assert_eq!((totals.volume_used_bytes, totals.apps.len()), (None, 0));
        }
    }

    #[test]
    fn android_background_task_platform_apps_node_has_no_location() {
        let tree = SizeNode {
            name: "0".into(),
            size: 100,
            is_dir: true,
            children: vec![SizeNode {
                name: "a.bin".into(),
                size: 100,
                is_dir: false,
                children: Vec::new(),
            }],
        };
        let args = json!({ "platform": { "volumeUsedBytes": 1000, "apps": [
            { "package": "app.own", "label": "Smart Explorer", "appBytes": 60, "dataBytes": 140 }
        ]}});
        let place = VolumeRoot::from_segments(&[], true);
        let approx = Approximations::compute(&tree, &place, platform_totals(&args), true);
        let task = "android-background-task-apps";
        let pending = Pending::open();
        bind_task(pending.token(), task);
        pending.store(Stored::Analysis {
            outcome: ScanOutcome::complete(tree),
            approx,
            base: "/storage/emulated/0".to_string(),
            root: "/storage/emulated/0".to_string(),
        });

        let root = node(&json!({ "taskId": task, "path": [] })).expect("root");
        assert_eq!(root["location"], "/storage/emulated/0");
        let row = root["children"]
            .as_array()
            .and_then(|rows| rows.iter().find(|row| row["kind"] == "apps"))
            .expect("apps row");
        let name = row["name"].as_str().expect("name");
        let list = node(&json!({ "taskId": task, "path": [name] })).expect("apps");
        assert!(list["location"].is_null());
        assert_eq!(
            (list["kind"].as_str(), list["size"].as_u64()),
            (Some("apps"), Some(200))
        );
        assert_eq!(list["children"][0]["package"], "app.own");
        assert_eq!(list["children"][0]["kind"], "app");
        assert!(node(&json!({ "taskId": task, "path": [name, "Smart Explorer"] })).is_err());
    }
}
