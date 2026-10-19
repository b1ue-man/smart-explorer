//! Signals of DRV-M6 for the names of the device tree: deterministic folder
//! names, the key of a device (contract V4) and the reserved folder `Räume`.
use super::device_tree_names::{
    device_key_of, folder_names, layout, room_folder_names, same_name, DEVICES_MOUNT_ID,
    ROOMS_FOLDER,
};
use super::device_tree_types::{DeviceKind, DeviceRow, RoomRef};
use super::mount_source::PeerMountTarget;
use super::path_dialect::PathDialect;

fn direct(contact: &str, name: &str) -> DeviceRow {
    let target = PeerMountTarget::Direct {
        contact_id: contact.into(),
    };
    DeviceRow {
        key: device_key_of(&target),
        name: name.into(),
        kind: DeviceKind::Direct,
        room: None,
        online: true,
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

fn names(rows: &[DeviceRow], dialect: PathDialect) -> Vec<String> {
    layout(rows, dialect)
        .direct
        .into_iter()
        .map(|entry| entry.name)
        .collect()
}

#[test]
fn drive_task_devices_names_collision_deterministic() {
    let rows = vec![
        direct("a", "Laptop"),
        direct("b", "laptop"),
        direct("c", "Büro"),
    ];
    // POSIX tells the two spellings apart: no suffix, byte order.
    assert_eq!(
        names(&rows, PathDialect::Posix),
        ["Büro", "Laptop", "laptop"]
    );
    // Windows compares by the ordinal upper-case form: both get the first
    // six hex digits of the SHA-256 of their key.
    assert_eq!(
        names(&rows, PathDialect::Windows),
        ["Büro", "Laptop (300164)", "laptop (cfdacd)"]
    );
    // The order of the input changes nothing.
    let mut reversed = rows.clone();
    reversed.reverse();
    for dialect in [PathDialect::Windows, PathDialect::Posix] {
        assert_eq!(layout(&rows, dialect), layout(&reversed, dialect));
        assert_eq!(
            folder_names(&rows, dialect),
            folder_names(&reversed, dialect)
        );
    }
    // Equal names under POSIX get the suffix as well, and stay valid names.
    let same = vec![direct("x", "Server"), direct("y", "Server")];
    assert_eq!(
        names(&same, PathDialect::Posix),
        ["Server (139dad)", "Server (946c35)"]
    );
    for name in names(&same, PathDialect::Windows) {
        assert!(PathDialect::Windows.validate_component(&name).is_ok());
    }
}

#[test]
fn drive_task_devices_key_matches_s5_rule() {
    let direct_target = PeerMountTarget::Direct {
        contact_id: "abc".into(),
    };
    assert_eq!(device_key_of(&direct_target), "direct~abc");
    let odd = PeerMountTarget::Direct {
        contact_id: "a/ü b".into(),
    };
    assert_eq!(device_key_of(&odd), "direct~a%2F%C3%BC%20b");
    let room = PeerMountTarget::RoomDevice {
        room_id: "r~1".into(),
        device_id: "d.1_-".into(),
    };
    assert_eq!(device_key_of(&room), "room~r%7E1~d.1_-");
    // The tiles of se Desktop and the tree use one function.
    let open = crate::share::PeerOpenTarget::RoomDevice {
        room_id: "r~1".into(),
        device_id: "d.1_-".into(),
    };
    assert_eq!(crate::mount::device_key(&open), "room~r%7E1~d.1_-");
    assert_eq!(
        crate::mount::mount_target_of(&open),
        room,
        "the conversion keeps both ids"
    );
    assert_eq!(crate::mount::open_target_of(&room), open);
    assert!(crate::mount::MountId::parse(DEVICES_MOUNT_ID).is_ok());
}

#[test]
fn drive_task_devices_names_reserved_rooms() {
    let rows = vec![
        direct("a", "Räume"),
        member("r1", "Team", "d1", "Laptop"),
        direct("b", "Notebook"),
    ];
    let windows = layout(&rows, PathDialect::Windows);
    let direct_names: Vec<&str> = windows.direct.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(direct_names, ["Notebook", "Räume (Gerät)"]);
    assert_eq!(windows.rooms.len(), 1);
    // Windows takes any letter case for the same name, POSIX does not.
    let lower = vec![direct("a", "räume"), member("r1", "Team", "d1", "Laptop")];
    assert_eq!(names(&lower, PathDialect::Windows), ["Räume (Gerät)"]);
    assert_eq!(names(&lower, PathDialect::Posix), ["räume"]);
    // Without a room there is no folder to clash with.
    let alone = vec![direct("a", "Räume")];
    assert!(layout(&alone, PathDialect::Posix).rooms.is_empty());
    // A device that already has the replacement name still stays unique.
    let clash = vec![direct("a", "Räume"), direct("b", "Räume (Gerät)")];
    let listed = names(&clash, PathDialect::Posix);
    assert_eq!(listed.len(), 2);
    assert_ne!(listed[0], listed[1]);
    assert!(listed.iter().all(|name| name != ROOMS_FOLDER));
    assert!(same_name(PathDialect::Windows, "RÄUME", ROOMS_FOLDER));
}

#[test]
fn devices_task_names_follow_the_dialect_rules() {
    let long = "a".repeat(300);
    let umlauts = "ä".repeat(200);
    let rows = vec![
        direct("1", "a:b?"),
        direct("2", "PC."),
        direct("3", "CON"),
        direct("4", "nul.txt"),
        direct("5", ""),
        direct("6", &long),
        direct("7", "COM¹"),
        direct("8", &umlauts),
    ];
    let windows = folder_names(&rows, PathDialect::Windows);
    assert_eq!(windows["direct~1"], "a_b_");
    assert_eq!(windows["direct~2"], "PC_");
    assert_eq!(windows["direct~3"], "CON_");
    // `nul.txt_` is still a DOS device name for the dialect: the probe of
    // rule 2 replaces it.
    assert!(windows["direct~4"].starts_with("Gerät "));
    assert_eq!(windows["direct~5"], "Gerät");
    assert_eq!(windows["direct~6"].len(), 255);
    assert!(windows["direct~7"].starts_with("Gerät "));
    assert_eq!(windows["direct~8"].chars().count(), 200);
    let posix = folder_names(&rows, PathDialect::Posix);
    assert_eq!(posix["direct~1"], "a:b?");
    assert_eq!(posix["direct~3"], "CON");
    assert_eq!(posix["direct~8"].len(), 254, "cut at a character boundary");
    for (key, name) in &windows {
        assert!(
            PathDialect::Windows.validate_component(name).is_ok(),
            "{key}: {name}"
        );
    }
    for (key, name) in &posix {
        assert!(
            PathDialect::Posix.validate_component(name).is_ok(),
            "{key}: {name}"
        );
    }
    let dots = vec![direct("9", ".."), direct("10", "a/b")];
    let posix = folder_names(&dots, PathDialect::Posix);
    assert_eq!(posix["direct~9"], "_");
    assert_eq!(posix["direct~10"], "a_b");
}

#[test]
fn devices_task_rooms_have_their_own_namespaces() {
    let rows = vec![
        member("r1", "Team", "d1", "Laptop"),
        member("r2", "Team", "d1", "Laptop"),
        member("r1", "Team", "d2", "Laptop"),
    ];
    let tree = layout(&rows, PathDialect::Posix);
    let room_names: Vec<&str> = tree.rooms.iter().map(|room| room.name.as_str()).collect();
    assert_eq!(room_names, ["Team (82f3e9)", "Team (db77fd)"]);
    let first: Vec<&str> = tree.rooms[0]
        .devices
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(first.len(), 2);
    assert!(first[0].starts_with("Laptop (") && first[1].starts_with("Laptop ("));
    let second: Vec<&str> = tree.rooms[1]
        .devices
        .iter()
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(second, ["Laptop"], "a room names its devices on its own");
    let folder = tree.folder_of("room~r2~d1").expect("folder");
    assert_eq!(folder, ["Räume", "Team (db77fd)", "Laptop"]);
    let rooms = [
        RoomRef {
            id: "r2".into(),
            name: "Team".into(),
        },
        RoomRef {
            id: "r1".into(),
            name: "Team".into(),
        },
    ];
    let by_id = room_folder_names(&rooms, PathDialect::Posix);
    assert_eq!(by_id["r1"], "Team (82f3e9)");
    assert_eq!(by_id["r2"], "Team (db77fd)");
}
