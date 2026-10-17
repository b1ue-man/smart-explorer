//! DSK-M1: arrangement per monitor (C5.4, C5.5, Ä27): new items on the
//! primary monitor, adoption of removed monitors, no vault paths.
use std::collections::BTreeMap;

use super::config::{DesktopConfig, SystemItems};
use super::items::{surface_items, visible_files, DesktopFile, SystemItem};
use super::layout::{Cell, DesktopLayout, ItemId, LayoutError, LayoutOp, MonitorLayout};
use super::state::Platform;
use super::tiles::{project, ShareState, TileInput, VaultTile};
use crate::share::ShareProfiles;

fn device(key: &str) -> ItemId {
    ItemId::Device { key: key.into() }
}

fn file(name: &str) -> ItemId {
    ItemId::File { name: name.into() }
}

fn cell(col: u32, row: u32) -> Cell {
    Cell { col, row }
}

fn desktop_file(name: &str, public: bool, hidden: bool) -> DesktopFile {
    DesktopFile {
        name: name.into(),
        is_dir: false,
        public,
        launcher: false,
        hidden,
    }
}

fn place(layout: &mut DesktopLayout, monitor: &str, item: ItemId, at: Cell) {
    layout.monitor_mut(monitor).unwrap().cells.insert(item, at);
}

#[test]
fn desktop_task_layout_per_monitor_new_items_on_primary() {
    let mut layout = DesktopLayout::default();
    place(&mut layout, "B", device("direct~c1"), cell(0, 0));
    place(&mut layout, "A", file("Bericht.md"), cell(0, 0));
    let items = [
        device("direct~c1"),
        file("Bericht.md"),
        device("direct~c2"),
        file("Notizen.txt"),
        ItemId::AllScreens,
    ];
    let placed = layout.place_new_items("A", &items).unwrap();
    assert_eq!(
        placed,
        [device("direct~c2"), file("Notizen.txt"), ItemId::AllScreens]
    );
    // Placed items keep their monitor; new ones fill the primary column.
    assert_eq!(layout.locate(&device("direct~c1")), Some(("B", cell(0, 0))));
    assert_eq!(layout.locate(&file("Bericht.md")), Some(("A", cell(0, 0))));
    assert_eq!(layout.locate(&device("direct~c2")), Some(("A", cell(0, 1))));
    assert_eq!(layout.locate(&file("Notizen.txt")), Some(("A", cell(0, 2))));
    assert_eq!(layout.locate(&ItemId::AllScreens), Some(("A", cell(0, 3))));
    let before = layout.clone();
    assert!(layout.place_new_items("A", &items).unwrap().is_empty());
    assert_eq!(layout, before);

    // Column by column: with two rows the third item opens column 1.
    let mut short = DesktopLayout::default();
    short
        .apply(&LayoutOp::Grid {
            monitor: "A".into(),
            rows: 2,
        })
        .unwrap();
    short
        .place_new_items("A", &[file("a"), file("b"), file("c")])
        .unwrap();
    assert_eq!(short.locate(&file("c")), Some(("A", cell(1, 0))));
    // Fewer rows later: items below the grid move to free cells.
    short
        .apply(&LayoutOp::Grid {
            monitor: "A".into(),
            rows: 1,
        })
        .unwrap();
    assert_eq!(short.locate(&file("a")), Some(("A", cell(0, 0))));
    assert_eq!(short.locate(&file("b")), Some(("A", cell(2, 0))));
    assert_eq!(short.locate(&file("c")), Some(("A", cell(1, 0))));

    // First activation on Windows: system items, then files by name.
    let files = [
        desktop_file("Zebra.txt", false, false),
        desktop_file("apfel.txt", false, false),
        desktop_file("Birne.txt", true, false),
        desktop_file("Birne.txt", false, false),
        desktop_file("desktop.ini", false, true),
    ];
    let names: Vec<(String, bool)> = visible_files(&files)
        .into_iter()
        .map(|file| (file.name, file.public))
        .collect();
    assert_eq!(
        names,
        [
            ("apfel.txt".to_string(), false),
            ("Birne.txt".to_string(), false),
            ("Birne.txt".to_string(), true),
            ("Zebra.txt".to_string(), false),
        ]
    );
    let config = DesktopConfig::initial(SystemItems::initial(None, None));
    let profiles = ShareProfiles::default();
    let (paths, state) = (BTreeMap::new(), ShareState::Live);
    let tiles = project(&TileInput {
        profiles: Ok(&profiles),
        share_state: &state,
        now_secs: 0,
        device_paths: &paths,
        vaults: None,
    });
    let order = surface_items(Platform::Windows, &config, &files, &tiles);
    assert_eq!(
        order,
        [
            ItemId::System(SystemItem::RecycleBin),
            file("apfel.txt"),
            file("Birne.txt"),
            ItemId::PublicFile {
                name: "Birne.txt".into()
            },
            file("Zebra.txt"),
            ItemId::AllScreens,
            ItemId::AddDevice,
            ItemId::JoinRoom,
        ]
    );
    // Linux has no system items.
    let linux = surface_items(Platform::Linux, &config, &files, &tiles);
    assert_eq!(linux.first(), Some(&file("apfel.txt")));

    // Drag on an occupied cell: the occupant moves to the first free cell.
    layout
        .apply(&LayoutOp::Move {
            item: device("direct~c2"),
            monitor: "A".into(),
            col: 0,
            row: 0,
        })
        .unwrap();
    assert_eq!(layout.locate(&device("direct~c2")), Some(("A", cell(0, 0))));
    assert_eq!(layout.locate(&file("Bericht.md")), Some(("A", cell(0, 1))));
    // "Auf Bildschirm verschieben": first free cell of the other monitor.
    layout
        .apply(&LayoutOp::MoveToMonitor {
            item: file("Notizen.txt"),
            monitor: "B".into(),
        })
        .unwrap();
    assert_eq!(layout.locate(&file("Notizen.txt")), Some(("B", cell(0, 1))));
    // An invalid monitor key changes nothing.
    let before = layout.clone();
    let invalid = LayoutOp::Move {
        item: device("direct~c2"),
        monitor: "../x".into(),
        col: 0,
        row: 0,
    };
    assert!(matches!(
        layout.apply(&invalid),
        Err(LayoutError::Monitor(_))
    ));
    assert_eq!(layout, before);
    // "Anordnen" places the given order column by column.
    layout
        .apply(&LayoutOp::Arrange {
            monitor: "A".into(),
            order: vec![ItemId::AllScreens, file("Bericht.md")],
        })
        .unwrap();
    assert_eq!(layout.locate(&ItemId::AllScreens), Some(("A", cell(0, 0))));
    assert_eq!(layout.locate(&file("Bericht.md")), Some(("A", cell(0, 1))));
    assert_eq!(layout.locate(&device("direct~c2")), Some(("A", cell(0, 2))));

    // One file per monitor, read back unchanged.
    let primary = &layout.monitors["A"];
    assert_eq!(primary.file_name(), "A.json");
    let text = primary.to_json().unwrap();
    assert_eq!(MonitorLayout::from_json(&text).as_ref(), Ok(primary));
    let duplicate = text.replace("\"row\": 1", "\"row\": 0");
    assert!(matches!(
        MonitorLayout::from_json(&duplicate),
        Err(LayoutError::Json(_))
    ));

    // Files that left the folder leave the arrangement; devices stay.
    layout.forget_missing_files(&[file("Bericht.md")]);
    assert!(layout.locate(&file("Notizen.txt")).is_none());
    assert!(layout.locate(&file("Bericht.md")).is_some());
    assert!(layout.locate(&device("direct~c1")).is_some());
}

#[test]
fn desktop_task_layout_monitor_removed_adopted_by_primary() {
    let mut layout = DesktopLayout::default();
    place(&mut layout, "A", device("direct~x"), cell(0, 0));
    place(&mut layout, "B", device("direct~y"), cell(0, 0));
    place(&mut layout, "B", device("direct~z"), cell(0, 1));
    place(&mut layout, "B", device("direct~w"), cell(1, 0));
    place(&mut layout, "C", device("direct~v"), cell(4, 2));
    let present = ["A".to_string(), "C".to_string()];
    let removed = layout.adopt_removed_monitors("A", &present).unwrap();
    // The os layer deletes `layout/B.json`.
    assert_eq!(removed, ["B".to_string()]);
    assert!(!layout.monitors.contains_key("B"));
    // B's items join A for good, in B's reading order.
    assert_eq!(layout.locate(&device("direct~x")), Some(("A", cell(0, 0))));
    assert_eq!(layout.locate(&device("direct~y")), Some(("A", cell(0, 1))));
    assert_eq!(layout.locate(&device("direct~z")), Some(("A", cell(0, 2))));
    assert_eq!(layout.locate(&device("direct~w")), Some(("A", cell(0, 3))));
    assert_eq!(layout.locate(&device("direct~v")), Some(("C", cell(4, 2))));
    // The primary monitor is never removed, even when it is not listed.
    let removed = layout
        .adopt_removed_monitors("A", &["C".to_string()])
        .unwrap();
    assert!(removed.is_empty());
    assert!(layout.monitors.contains_key("A"));
}

#[test]
fn desktop_task_layout_has_no_vault_paths() {
    let vaults = [VaultTile {
        site: "3f2a9c1b".into(),
        label: "Steuer 2025".into(),
        state: "unlocked".into(),
    }];
    let profiles = ShareProfiles::default();
    let (paths, state) = (BTreeMap::new(), ShareState::Live);
    let tiles = project(&TileInput {
        profiles: Ok(&profiles),
        share_state: &state,
        now_secs: 0,
        device_paths: &paths,
        vaults: Some(&vaults),
    });
    let config = DesktopConfig::initial(SystemItems::initial(None, None));
    let items = surface_items(Platform::Linux, &config, &[], &tiles);
    assert!(items.contains(&ItemId::Vault {
        site: "3f2a9c1b".into()
    }));
    let mut layout = DesktopLayout::default();
    layout.place_new_items("eDP-1", &items).unwrap();
    let text = layout.monitors["eDP-1"].to_json().unwrap();
    assert!(text.contains("\"vault:3f2a9c1b\""));
    for secret in ["Steuer", "2025", "unlocked", "Tresore", "/run/user"] {
        assert!(!text.contains(secret), "{secret} in {text}");
    }
    // Every item ID round-trips; vault IDs carry only the site ID.
    let ids = [
        device("direct~c1"),
        device("room~r~d"),
        ItemId::Room {
            room_id: "r".into(),
        },
        ItemId::Vault {
            site: "3f2a9c1b".into(),
        },
        file("Bericht.md"),
        ItemId::PublicFile {
            name: "Handbuch.pdf".into(),
        },
        ItemId::System(SystemItem::RecycleBin),
        ItemId::System(SystemItem::ThisPc),
        ItemId::AllScreens,
        ItemId::AddDevice,
        ItemId::JoinRoom,
    ];
    for id in ids {
        assert_eq!(ItemId::parse(&id.encode()), Ok(id.clone()));
    }
    for bad in [
        "",
        "vault:",
        "tresor:x",
        "system:papierkorb",
        "screens:alle",
        "nix",
    ] {
        assert!(ItemId::parse(bad).is_err(), "{bad}");
    }
    // Hidden vault group: no vault item at all.
    let mut hidden = config.clone();
    hidden.groups.vaults = false;
    assert!(!surface_items(Platform::Linux, &hidden, &[], &tiles)
        .iter()
        .any(|item| matches!(item, ItemId::Vault { .. })));
}
