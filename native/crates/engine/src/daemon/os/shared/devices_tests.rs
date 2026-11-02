//! Signals of DRV-M6 for the daemon side of the device tree: the shape of
//! the status, the watch, the gate for restricted workers, the default
//! „ein“, switching off and the unchanged single-peer drive.
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::super::super::ipc_protocol::IpcResponse;
use super::super::devices_host::{child_gate, wait_for_change, Board, DEFERRED_CODE};
use super::super::devices_mount::{
    apply_default_with, forget_in, read_choice_in, write_choice_in, Choice, DefaultWorld,
};
use super::super::ipc_protocol_mount_host::MAX_WATCH_MS;
use super::{build_status, DevicesStatus};
use crate::daemon::MountHostConfig;
use crate::mount::{
    device_key_of, BackendRoot, DeviceKind, DeviceRow, DriveLetter, DriveSelection, MountConfig,
    MountId, MountMode, MountRecovery, MountSnapshot, MountSource, MountStatus, PathDialect,
    PeerMountTarget, RoomRef, DEVICES_MOUNT_ID,
};

fn direct(contact: &str, name: &str, online: bool) -> DeviceRow {
    let target = PeerMountTarget::Direct {
        contact_id: contact.into(),
    };
    DeviceRow {
        key: device_key_of(&target),
        name: name.into(),
        kind: DeviceKind::Direct,
        room: None,
        online,
        target,
    }
}

fn member(room: &str, room_name: &str, device: &str, name: &str) -> DeviceRow {
    let target = PeerMountTarget::RoomDevice {
        room_id: room.into(),
        device_id: device.into(),
    };
    DeviceRow {
        key: device_key_of(&target),
        name: name.into(),
        kind: DeviceKind::Room,
        room: Some(RoomRef {
            id: room.into(),
            name: room_name.into(),
        }),
        online: true,
        target,
    }
}

/// The folder of the device tree: an absolute path of the running platform
/// (a location without a drive letter is not absolute on Windows).
fn tree_location() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\Users\u\SmartExplorer\Geräte")
    } else {
        PathBuf::from("/home/u/SmartExplorer/Geräte")
    }
}

fn tree_snapshot(status: MountStatus) -> MountSnapshot {
    let config = MountConfig::new(
        MountId::parse(DEVICES_MOUNT_ID).unwrap(),
        MountSource::Devices,
        DriveSelection::Automatic,
        MountMode::ReadWrite,
        crate::mount::devices_label(),
    )
    .unwrap()
    .with_location_directory(Some(tree_location()))
    .unwrap();
    MountSnapshot {
        capacity: None,
        config,
        status,
        recovery: MountRecovery::Clean,
        recovery_required_compat: false,
    }
}

#[test]
fn drive_task_devices_status_json_shape() {
    let rows = vec![
        direct("a", "Laptop", true),
        member("r1", "Team", "d1", "Raum"),
    ];
    let mounted = tree_snapshot(MountStatus::MountedAt {
        path: PathBuf::from("/home/u/SmartExplorer/Geräte"),
    });
    let status = build_status(Some(&mounted), &rows, 2, PathDialect::Posix);
    let value = serde_json::to_value(&status).unwrap();
    let object = value.as_object().unwrap();
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    let mut expected = vec![
        "devices",
        "enabled",
        "mounted",
        "pending",
        "revision",
        "root_path",
        "state",
    ];
    if cfg!(target_os = "linux") {
        expected.push("mount_point");
        expected.sort_unstable();
    }
    assert_eq!(keys, expected, "no letter on Linux, nothing written twice");
    assert_eq!(value["state"], "mounted");
    assert_eq!(value["mounted"], true);
    assert_eq!(value["enabled"], true);
    assert_eq!(value["root_path"], "/home/u/SmartExplorer/Geräte");
    assert_eq!(value["pending"], 2);
    let devices = value["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 2);
    assert_eq!(
        devices[0],
        json!({
            "key": "direct~a",
            "name": "Laptop",
            "kind": "direct",
            "online": true,
            "path": "/home/u/SmartExplorer/Geräte/Laptop"
        })
    );
    assert_eq!(
        devices[1],
        json!({
            "key": "room~r1~d1",
            "name": "Raum",
            "kind": "room",
            "room": "Team",
            "online": true,
            "path": "/home/u/SmartExplorer/Geräte/Räume/Team/Raum"
        })
    );
    // The answer survives its own wire form.
    let again: DevicesStatus = serde_json::from_value(value).unwrap();
    assert_eq!(again, status);
}

#[test]
fn devices_task_status_states_and_paths() {
    let rows = vec![
        direct("a", "Laptop", false),
        member("r1", "Team", "d1", "Raum"),
    ];
    // Off: no entry, no path, but the devices are listed.
    let off = build_status(None, &rows, 0, PathDialect::Posix);
    assert!(!off.enabled && !off.mounted && off.state == "off");
    assert!(off.devices.iter().all(|device| device.path.is_none()));
    assert!(off.root_path.is_none() && off.letter.is_none() && off.mount_point.is_none());
    // Windows: a letter and `X:\` paths.
    let drive = DriveLetter::parse('x').unwrap();
    let windows = build_status(
        Some(&tree_snapshot(MountStatus::Mounted { drive })),
        &rows,
        0,
        PathDialect::Windows,
    );
    assert_eq!(windows.letter.as_deref(), Some("X:"));
    assert_eq!(windows.root_path.as_deref(), Some("X:\\"));
    assert_eq!(windows.devices[0].path.as_deref(), Some("X:\\Laptop"));
    assert_eq!(
        windows.devices[1].path.as_deref(),
        Some("X:\\Räume\\Team\\Raum")
    );
    // The other states.
    let state = |status: MountStatus| {
        build_status(Some(&tree_snapshot(status)), &[], 0, PathDialect::Posix).state
    };
    assert_eq!(state(MountStatus::Mounting), "mounting");
    assert_eq!(state(MountStatus::Unmounting), "unmounting");
    assert_eq!(state(MountStatus::Unmounted), "stopped");
    assert_eq!(
        state(MountStatus::Failed {
            detail: "Wartet auf Smart Explorer Share: Laufwerke anderer Geräte werden eingebunden"
                .into()
        }),
        "waiting"
    );
    assert_eq!(
        state(MountStatus::Failed {
            detail: "FUSE fehlt".into()
        }),
        "failed"
    );
    assert_eq!(
        state(MountStatus::RuntimeUnavailable {
            detail: "Dokany fehlt".into()
        }),
        "failed"
    );
}

#[test]
fn drive_task_devices_watch_returns_within_30s() {
    // A new revision ends the wait at once.
    let mut polls = 0;
    let started = Instant::now();
    let value = wait_for_change(4, Duration::from_secs(5), Duration::from_millis(5), || {
        polls += 1;
        (if polls >= 3 { 5 } else { 4 }, polls)
    });
    assert_eq!(value, 3);
    assert!(started.elapsed() < Duration::from_secs(2));
    // Without a change it ends at the longest wait with the current value.
    let started = Instant::now();
    let value = wait_for_change(
        4,
        Duration::from_millis(60),
        Duration::from_millis(10),
        || (4, "unchanged"),
    );
    assert_eq!(value, "unchanged");
    assert!(started.elapsed() >= Duration::from_millis(60));
    assert!(started.elapsed() < Duration::from_secs(2));
    // The longest wait stays under the 35 s read deadline of the clients
    // and the 50 s of the IPC.
    assert_eq!(MAX_WATCH_MS, 30_000);
    // The first observation is revision 1; the same value keeps it.
    let board: Board<Vec<u32>> = Board::new();
    assert_eq!(board.observe(&vec![1]), 1);
    assert_eq!(board.observe(&vec![1]), 1);
    assert_eq!(board.observe(&vec![1, 2]), 2);
}

#[test]
fn drive_task_devices_child_waits_for_share_allowed() {
    // A worker started only for the drive restore has no Share host: the
    // host is told to wait, with a code it can tell from a real failure.
    match child_gate(false) {
        Some(IpcResponse::Err { msg, code }) => {
            assert_eq!(code.as_deref(), Some("share_deferred"));
            assert_eq!(code.as_deref(), Some(DEFERRED_CODE));
            assert!(msg.starts_with("Wartet auf Smart Explorer Share"));
        }
        other => panic!("unexpected gate {other:?}"),
    }
    assert!(child_gate(true).is_none());
}

#[derive(Default)]
struct World {
    decided: bool,
    registered: bool,
    supported: bool,
    runtime: bool,
    devices: bool,
    fail: bool,
    enabled: Cell<usize>,
    reported: Cell<usize>,
}

impl World {
    fn ready() -> Self {
        Self {
            supported: true,
            runtime: true,
            devices: true,
            ..Self::default()
        }
    }
}

impl DefaultWorld for World {
    fn decided(&self) -> bool {
        self.decided
    }

    fn registered(&self) -> bool {
        self.registered
    }

    fn supported(&self) -> bool {
        self.supported
    }

    fn runtime_ready(&self) -> bool {
        self.runtime
    }

    fn has_devices(&self) -> bool {
        self.devices
    }

    fn enable(&self) -> Result<(), String> {
        self.enabled.set(self.enabled.get() + 1);
        if self.fail {
            Err("kein Einhängeort".into())
        } else {
            Ok(())
        }
    }

    fn report(&self, _text: &str) {
        self.reported.set(self.reported.get() + 1);
    }
}

#[test]
fn drive_task_devices_enable_default_applies_once() {
    // Everything is there: exactly one switch-on, however often it is asked.
    let world = World::ready();
    let tried = AtomicBool::new(false);
    apply_default_with(&tried, &world);
    apply_default_with(&tried, &world);
    assert_eq!(world.enabled.get(), 1);
    // After the user switched off (a decision exists) it never comes back.
    let off = World {
        decided: true,
        ..World::ready()
    };
    apply_default_with(&AtomicBool::new(false), &off);
    assert_eq!(off.enabled.get(), 0);
    // Each missing condition keeps it off.
    for world in [
        World {
            registered: true,
            ..World::ready()
        },
        World {
            supported: false,
            ..World::ready()
        },
        World {
            runtime: false,
            ..World::ready()
        },
        World {
            devices: false,
            ..World::ready()
        },
    ] {
        apply_default_with(&AtomicBool::new(false), &world);
        assert_eq!(world.enabled.get(), 0);
    }
    // A failure is reported; the decision stays unwritten (enable writes
    // it only on success), so the next service start tries again.
    let failing = World {
        fail: true,
        ..World::ready()
    };
    apply_default_with(&AtomicBool::new(false), &failing);
    assert_eq!((failing.enabled.get(), failing.reported.get()), (1, 1));
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(read_choice_in(directory.path()), None);
}

#[test]
fn drive_task_devices_disable_keeps_child_journals() {
    let directory = tempfile::tempdir().unwrap();
    let id = MountId::parse(DEVICES_MOUNT_ID).unwrap();
    let config = tree_snapshot(MountStatus::Unmounted).config;
    super::super::super::mount_registry::upsert_in(directory.path(), &config).unwrap();
    write_choice_in(directory.path(), Choice::On).unwrap();
    // The caches of the children and the list of them, as a host leaves
    // them when changes of a device are still unsaved.
    let cache = directory.path().join("mount-cache");
    std::fs::create_dir_all(cache.join(DEVICES_MOUNT_ID)).unwrap();
    std::fs::create_dir_all(cache.join("0123456789abcdef0123456789abcdef")).unwrap();
    std::fs::write(cache.join("0123456789abcdef0123456789abcdef/journal"), b"x").unwrap();
    std::fs::write(
        cache.join(DEVICES_MOUNT_ID).join("children.json"),
        br#"{"version":1,"children":[]}"#,
    )
    .unwrap();
    forget_in(directory.path(), &id).unwrap();
    // The registry entry is gone and the decision is „aus“ …
    let left = super::super::super::mount_registry::load_in(directory.path()).unwrap();
    assert!(left.is_empty(), "the registry keeps no device tree");
    assert_eq!(read_choice_in(directory.path()), Some(Choice::Off));
    // … and the journals of the children are untouched.
    assert!(cache
        .join("0123456789abcdef0123456789abcdef/journal")
        .exists());
    assert!(cache.join(DEVICES_MOUNT_ID).join("children.json").exists());
}

/// The place in the file manager goes with the drive (Linux only: the other
/// platforms have none), and the registry forgets the tree in every case.
#[cfg(target_os = "linux")]
fn places_go_with_the_drive() {
    use crate::mount::os::linux_os::bookmarks::{add_in, remove_in, BookmarkEntry, Targets};
    let home = tempfile::tempdir().unwrap();
    let targets = Targets::resolve(
        Some(home.path().join("config").into_os_string()),
        Some(home.path().join("data").into_os_string()),
        Some(home.path().join("home")),
    )
    .unwrap();
    let folder = home.path().join("home/SmartExplorer/Geräte");
    std::fs::create_dir_all(folder.parent().unwrap()).unwrap();
    let entry = BookmarkEntry {
        id: DEVICES_MOUNT_ID.into(),
        folder: folder.clone(),
        label: "Geräte".into(),
    };
    add_in(&targets, &entry).unwrap();
    let kde = |targets: &Targets| std::fs::read_to_string(&targets.kde).unwrap();
    assert!(kde(&targets).contains("urn:smart-explorer:places"));
    // The host removes its place when it ends; so does the cleanup of the
    // daemon after a host that died. Both leave no entry of the tree.
    remove_in(&targets, &folder).unwrap();
    assert!(!kde(&targets).contains("urn:smart-explorer:places"));
}

#[cfg(not(target_os = "linux"))]
fn places_go_with_the_drive() {}

#[test]
fn drive_task_devices_switch_off_unmounts_and_removes_place() {
    places_go_with_the_drive();
    let directory = tempfile::tempdir().unwrap();
    let id = MountId::parse(DEVICES_MOUNT_ID).unwrap();
    let config = tree_snapshot(MountStatus::Unmounted).config;
    super::super::super::mount_registry::upsert_in(directory.path(), &config).unwrap();
    forget_in(directory.path(), &id).unwrap();
    assert!(
        super::super::super::mount_registry::load_in(directory.path())
            .unwrap()
            .is_empty()
    );
    assert_eq!(read_choice_in(directory.path()), Some(Choice::Off));
}

#[test]
fn drive_task_devices_single_peer_mounts_unchanged() {
    // A drive of one peer keeps the bytes of every earlier release: the
    // host configuration carries no `device_tree`, the drive does not
    // restore at login by itself and it is no tree.
    let peer = MountConfig::new(
        MountId::parse("peer-drive").unwrap(),
        MountSource::Peer {
            target: PeerMountTarget::Direct {
                contact_id: "c1".into(),
            },
            root: BackendRoot::parse("/Docs").unwrap(),
        },
        DriveSelection::Automatic,
        MountMode::ReadOnly,
        "Docs",
    )
    .unwrap();
    assert!(!peer.source.is_device_tree());
    assert!(!peer.restores_at_login());
    assert!(peer.source.stored_in_first_registry());
    let host: Value = serde_json::to_value(MountHostConfig::from(&peer)).unwrap();
    assert!(host.get("device_tree").is_none(), "{host}");
    // The tree is the one source that sets it, and that restores at login.
    let tree = tree_snapshot(MountStatus::Unmounted).config;
    let host: Value = serde_json::to_value(MountHostConfig::from(&tree)).unwrap();
    assert_eq!(host["device_tree"], true);
    assert!(tree.restores_at_login());
    assert!(!tree.source.stored_in_first_registry());
}
