//! DSK-M4: the shell is the only writer of `layout/*.json`. Two surfaces send
//! arrangement changes at the same time; the shell applies them one after
//! the other and every monitor ends with exactly one valid file.
use std::sync::mpsc;

use super::{forget_missing_files, LayoutStore};
use crate::desktop::core::items::DesktopFile;
use crate::desktop::core::layout::{Cell, ItemId, LayoutOp, MonitorLayout};
use crate::desktop::os::store::DesktopDir;

fn file_names(dir: &DesktopDir) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir.layout_dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn desktop_task_layout_single_writer() {
    let root = tempfile::tempdir().unwrap();
    let dir = DesktopDir::at(root.path().join("desktop"));
    dir.create().unwrap();
    let (mut store, mut layout) = LayoutStore::open(dir.clone());
    assert!(layout.monitors.is_empty());
    let devices: Vec<ItemId> = (0..12)
        .map(|index| ItemId::Device {
            key: format!("direct~c{index}"),
        })
        .collect();
    layout.place_new_items("mon-a", &devices).unwrap();
    layout.monitor_mut("mon-b").unwrap();
    assert_eq!(store.save(&layout).unwrap(), 2);
    assert_eq!(store.save(&layout).unwrap(), 0, "nothing changed");

    // Two surfaces send at the same time; the shell applies every change in
    // its loop and writes after each (the shell is the only writer).
    let (sender, changes) = mpsc::channel::<LayoutOp>();
    let surfaces: Vec<std::thread::JoinHandle<()>> = [("mon-a", 0usize), ("mon-b", 6usize)]
        .into_iter()
        .map(|(monitor, first)| {
            let sender = sender.clone();
            let items = devices.clone();
            std::thread::spawn(move || {
                for round in 0..40u32 {
                    let item = items[first + round as usize % 6].clone();
                    let monitor = monitor.to_string();
                    let op = if round.is_multiple_of(5) {
                        LayoutOp::MoveToMonitor { item, monitor }
                    } else {
                        LayoutOp::Move {
                            item,
                            monitor,
                            col: round % 4,
                            row: round % 3,
                        }
                    };
                    sender.send(op).unwrap();
                }
            })
        })
        .collect();
    drop(sender);
    for op in changes {
        layout.apply(&op).unwrap();
        store.save(&layout).unwrap();
    }
    for surface in surfaces {
        surface.join().unwrap();
    }
    // One valid file per monitor with the shell's arrangement, no temporary
    // file left behind.
    assert_eq!(file_names(&dir), ["mon-a.json", "mon-b.json"]);
    for (key, monitor) in &layout.monitors {
        let text = std::fs::read_to_string(dir.layout_dir().join(format!("{key}.json"))).unwrap();
        assert_eq!(&MonitorLayout::from_json(&text).unwrap(), monitor);
    }
    let reloaded = LayoutStore::open(dir.clone()).1;
    assert_eq!(reloaded, layout);

    // A monitor that left: its items go to the primary monitor, its file goes.
    layout
        .adopt_removed_monitors("mon-a", &["mon-a".to_string()])
        .unwrap();
    assert_eq!(store.save(&layout).unwrap(), 2);
    assert_eq!(file_names(&dir), ["mon-a.json"]);

    // The shell saves once per model revision; a file that left the desktop
    // folder leaves the arrangement, devices keep their cells.
    let note = ItemId::File {
        name: "notiz.txt".into(),
    };
    layout.place_new_items("mon-a", &[note.clone()]).unwrap();
    assert_eq!(store.save_at(&layout, 7).unwrap(), 1);
    assert_eq!(store.save_at(&layout, 7).unwrap(), 0);
    forget_missing_files(&mut layout, &[] as &[DesktopFile]);
    assert!(layout.locate(&note).is_none());
    assert!(layout.locate(&devices[0]).is_some());
    assert_eq!(store.save_at(&layout, 8).unwrap(), 1);

    // A file that names another monitor is not loaded and stays untouched.
    let foreign = dir.layout_dir().join("mon-c.json");
    let mut other = MonitorLayout::new("mon-d", 4).unwrap();
    other
        .cells
        .insert(ItemId::AllScreens, Cell { col: 0, row: 0 });
    std::fs::write(&foreign, other.to_json().unwrap()).unwrap();
    let (_, loaded) = LayoutStore::open(dir.clone());
    assert!(!loaded.monitors.contains_key("mon-c"));
    assert!(!loaded.monitors.contains_key("mon-d"));
    assert!(foreign.is_file());

    // Removed se Desktop: the store never creates the folder again.
    dir.delete_all().unwrap();
    layout.monitor_mut("mon-e").unwrap();
    assert_eq!(store.save(&layout).unwrap(), 0);
    assert!(!dir.exists());
}
