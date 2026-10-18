//! DSK-M4 sources of the shell: the answers of the device tree as contract
//! V4 describes them (Annahme until DRV-M6), the vault tiles of the vault
//! status and the desktop folders.
use serde_json::json;

use super::{
    device_tree_view, normalize_key, parse_device_tree, scan_folders, vault_tiles, DesktopFolder,
    DeviceTreeView,
};

#[test]
fn feed_sources_read_device_tree_answers() {
    let status = json!({
        "enabled": true, "state": "mounted", "mounted": true,
        "root_path": "/home/nutzer/SmartExplorer/Geräte", "letter": null,
        "mount_point": "/home/nutzer/SmartExplorer/Geräte", "revision": 7,
        "devices": [
            {"key": "direct~c1", "name": "Laptop", "kind": "direct", "room": null,
             "online": true, "path": "/home/nutzer/SmartExplorer/Geräte/Laptop"},
            {"key": "room~r%c3%a4~d1", "name": "Büro-PC", "kind": "room", "room": "Rä",
             "online": false, "path": "/home/nutzer/SmartExplorer/Geräte/Räume/Rä/Büro-PC"},
            {"key": "direct~c2", "name": "ohne Ordner", "kind": "direct", "room": null,
             "online": false, "path": ""}
        ]
    });
    let tree = parse_device_tree(&status).unwrap();
    assert!(tree.enabled && tree.mounted);
    assert_eq!(tree.revision, 7);
    assert_eq!(
        tree.root_path.as_deref(),
        Some("/home/nutzer/SmartExplorer/Geräte")
    );
    assert_eq!(tree.paths.len(), 2);
    assert_eq!(
        tree.paths.get("room~r%C3%A4~d1").map(String::as_str),
        Some("/home/nutzer/SmartExplorer/Geräte/Räume/Rä/Büro-PC")
    );
    // The same object inside one field of the answer envelope.
    let wrapped = json!({"t": "devices_status", "status": status.clone()});
    assert_eq!(parse_device_tree(&wrapped), Some(tree.clone()));
    // Folders count only while the tree is mounted.
    let view = DeviceTreeView::Known(tree.clone());
    assert!(view.mounted());
    assert_eq!(view.root_path(), Some("/home/nutzer/SmartExplorer/Geräte"));
    assert_eq!(view.mounted_paths().map(|paths| paths.len()), Some(2));
    let mut unmounted = tree;
    unmounted.mounted = false;
    let view = DeviceTreeView::Known(unmounted);
    assert!(view.mounted_paths().is_none());
    assert_eq!(view.root_path(), None);
    // No worker, a refusal and a foreign answer.
    assert_eq!(device_tree_view(Ok(None)), DeviceTreeView::NotRunning);
    assert!(matches!(
        device_tree_view(Err("unbekannt".into())),
        DeviceTreeView::Unknown(_)
    ));
    assert!(matches!(
        device_tree_view(Ok(Some(json!({"t": "ok"})))),
        DeviceTreeView::Unknown(_)
    ));
    assert_eq!(normalize_key("room~a%2fb~c%7E"), "room~a%2Fb~c%7E");
    assert_eq!(normalize_key("direct~x%"), "direct~x%");
}

#[test]
fn feed_sources_vault_tiles_keep_id_label_state() {
    let site_a = "0123456789abcdef0123456789abcdef";
    let site_b = "fedcba9876543210fedcba9876543210";
    let site =
        |id: &str, label: &str, state: &str| (id.to_string(), label.to_string(), state.to_string());
    let tiles = vault_tiles([
        site(site_a, "Privat", "locked"),
        site(site_b, " ", "unlocked"),
        site(site_a, "doppelt", "unlocked"),
        site("c0ffee00", "Alt", "unavailable"),
        site("../böse", "Weg", "locked"),
        site("", "leer", "locked"),
    ]);
    let shown: Vec<(&str, &str, &str)> = tiles
        .iter()
        .map(|tile| (tile.site.as_str(), tile.label.as_str(), tile.state.as_str()))
        .collect();
    assert_eq!(
        shown,
        [
            (site_a, "Privat", "locked"),
            (site_b, site_b, "unlocked"),
            ("c0ffee00", "Alt", "unavailable"),
        ]
    );
    // Only the site ID reaches a locator (S5-V4).
    assert_eq!(tiles[0].location(), format!("vault://{site_a}/"));
}

#[test]
fn feed_sources_scan_desktop_folders() {
    let root = tempfile::tempdir().unwrap();
    let user = root.path().join("Desktop");
    let public = root.path().join("Public");
    std::fs::create_dir_all(user.join("Projekte")).unwrap();
    std::fs::create_dir_all(&public).unwrap();
    for name in ["notiz.txt", ".versteckt", "desktop.ini", "start.desktop"] {
        std::fs::write(user.join(name), b"x").unwrap();
    }
    std::fs::write(public.join("Gemeinsam.lnk"), b"x").unwrap();
    let folders = [
        DesktopFolder {
            path: user,
            public: false,
            launchers: true,
        },
        DesktopFolder {
            path: public,
            public: true,
            launchers: false,
        },
        DesktopFolder {
            path: root.path().join("fehlt"),
            public: false,
            launchers: false,
        },
    ];
    let mut files = scan_folders(&folders);
    files.sort_by(|left, right| left.name.cmp(&right.name));
    let shown: Vec<(&str, bool, bool, bool, bool)> = files
        .iter()
        .map(|file| {
            (
                file.name.as_str(),
                file.is_dir,
                file.public,
                file.launcher,
                file.hidden,
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            (".versteckt", false, false, false, true),
            ("Gemeinsam.lnk", false, true, false, false),
            ("Projekte", true, false, false, false),
            ("desktop.ini", false, false, false, true),
            ("notiz.txt", false, false, false, false),
            ("start.desktop", false, false, true, false),
        ]
    );
}
