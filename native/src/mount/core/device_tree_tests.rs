//! Signals of DRV-M6 for the device tree, over local folders instead of
//! devices (no network): folders without manual steps, offline devices,
//! the journal that is published when a device comes back, read-only rooms,
//! moves between shared folders, idle engines and the start of a host with
//! devices that cannot be reached.
use std::io;
use std::sync::atomic::Ordering;

use super::super::frontend::MountFilesystem;
use super::super::path_dialect::PathDialect;
use super::super::types::{MountMode, OpenDisposition, OpenFileOptions};
use super::support::{
    create_file, create_options, direct, names, open_existing, wait_until, Env, Spec, LAPTOP,
};

#[test]
fn drive_task_devices_folder_appears_without_manual_steps() {
    let env = Env::new(&[
        direct("a", "Laptop", &["Docs", "Music", "Verbindungen/Srv"]),
        direct("b", "Büro", &["Docs"]),
    ]);
    std::fs::write(env.share_file(LAPTOP, "Docs/readme.txt"), b"hi").unwrap();
    // Nothing but the device list: the folders are there.
    env.publish();
    let tree = env.tree();
    assert_eq!(names(tree.list_dir("/").unwrap()), ["Büro", "Laptop"]);
    assert_eq!(
        names(tree.list_dir("/Laptop").unwrap()),
        ["Docs", "Music", "Verbindungen"]
    );
    assert_eq!(
        names(tree.list_dir("/Laptop/Verbindungen").unwrap()),
        ["Srv"]
    );
    assert!(tree.stat("/Laptop").unwrap().is_dir);
    // Naming folders opens no connection; a file does.
    assert_eq!(env.link.connects.load(Ordering::SeqCst), 0);
    assert_eq!(
        names(tree.list_dir("/Laptop/Docs").unwrap()),
        ["readme.txt"]
    );
    assert_eq!(env.link.connects.load(Ordering::SeqCst), 1);
    let handle = tree
        .open_file("/Laptop/Docs/readme.txt", open_existing())
        .unwrap();
    let mut buffer = [0u8; 8];
    assert_eq!(tree.read(handle, 0, &mut buffer).unwrap(), 2);
    assert_eq!(&buffer[..2], b"hi");
    tree.close(handle).unwrap();
    // The folders are changes the frontend learns about.
    let changes = tree.drain_metadata_changes(100).unwrap();
    assert!(changes.iter().any(|change| change.path() == "/Laptop"));
    assert!(changes.iter().any(|change| change.path() == "/Büro"));
}

#[test]
fn drive_task_devices_offline_error_not_empty() {
    let mut env = Env::new(&[direct("a", "Laptop", &["Docs"])]);
    env.set_online(LAPTOP, false);
    let tree = env.tree();
    // The device is listed, its content is an error, never an empty folder.
    assert_eq!(names(tree.list_dir("/").unwrap()), ["Laptop"]);
    let error = tree.list_dir("/Laptop").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::HostUnreachable);
    env.set_online(LAPTOP, true);
    let tree = env.tree();
    assert_eq!(names(tree.list_dir("/Laptop").unwrap()), ["Docs"]);
    assert!(tree.list_dir("/Laptop/Docs").unwrap().is_empty());
    // Offline again: the names stay, the content is an error.
    env.set_online(LAPTOP, false);
    let tree = env.tree();
    tree.tick();
    assert_eq!(names(tree.list_dir("/Laptop").unwrap()), ["Docs"]);
    assert!(tree.stat("/Laptop/Docs").unwrap().is_dir);
    let error = tree.list_dir("/Laptop/Docs").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::HostUnreachable);
}

/// Leaves one unpublished change in the journal of `/Laptop/Docs`.
fn leave_pending(env: &Env) {
    let tree = env.tree();
    assert!(tree.list_dir("/Laptop/Docs").unwrap().is_empty());
    let handle = tree
        .open_file("/Laptop/Docs/up.txt", create_options())
        .unwrap();
    tree.write(handle, 0, b"hello").unwrap();
    env.device(LAPTOP).down.store(true, Ordering::SeqCst);
    let _ = tree.flush(handle);
    let _ = tree.close(handle);
    assert!(!tree.dirty_entries().unwrap().is_empty());
    assert!(!env.share_file(LAPTOP, "Docs/up.txt").exists());
}

fn published(env: &Env) -> bool {
    std::fs::read(env.share_file(LAPTOP, "Docs/up.txt")).is_ok_and(|bytes| bytes == b"hello")
        && env
            .tree()
            .dirty_entries()
            .is_ok_and(|entries| entries.is_empty())
}

#[test]
fn drive_task_devices_online_retries_pending_upload() {
    let mut env = Env::new(&[direct("a", "Laptop", &["Docs"])]);
    env.publish();
    leave_pending(&env);
    // Offline: the journal keeps its engine open.
    env.set_online(LAPTOP, false);
    env.tree().tick();
    assert_eq!(env.tree().open_children().len(), 1);
    assert!(!env.tree().dirty_entries().unwrap().is_empty());
    // The device comes back: the journal is published without a user step.
    env.device(LAPTOP).down.store(false, Ordering::SeqCst);
    env.set_online(LAPTOP, true);
    env.tree().tick();
    wait_until("the upload after the device came back", || published(&env));
}

#[test]
fn drive_task_devices_offline_peer_does_not_block_host_start() {
    let mut env = Env::new(&[direct("a", "Laptop", &["Docs"])]);
    env.publish();
    leave_pending(&env);
    env.set_online(LAPTOP, false);
    let (connects, roots) = (
        env.link.connects.load(Ordering::SeqCst),
        env.link.roots.load(Ordering::SeqCst),
    );
    // A new host over the same cache: the device is offline, its journal is
    // found again, and nothing touches the network.
    env.restart();
    env.publish();
    env.tree().tick();
    assert_eq!(names(env.tree().list_dir("/").unwrap()), ["Laptop"]);
    assert!(!env.tree().dirty_entries().unwrap().is_empty());
    assert_eq!(env.link.connects.load(Ordering::SeqCst), connects);
    assert_eq!(env.link.roots.load(Ordering::SeqCst), roots);
    assert_eq!(crate::mount::pending_devices(&env.spool), 1);
    // The device returns: the recovered journal is published.
    env.device(LAPTOP).down.store(false, Ordering::SeqCst);
    env.set_online(LAPTOP, true);
    env.tree().tick();
    wait_until("the recovered upload", || published(&env));
}

#[test]
fn drive_task_devices_offline_closes_idle_children_only() {
    let mut env = Env::new(&[direct("a", "Laptop", &["Docs", "Music"])]);
    std::fs::write(env.share_file(LAPTOP, "Music/song.txt"), b"la").unwrap();
    env.publish();
    let tree = env.tree();
    tree.list_dir("/Laptop/Docs").unwrap();
    tree.list_dir("/Laptop/Music").unwrap();
    assert_eq!(tree.open_children().len(), 2);
    assert_eq!(env.link.leases(), 2);
    let handle = tree
        .open_file("/Laptop/Music/song.txt", open_existing())
        .unwrap();
    // Offline: the idle engine goes at once, the one with an open file stays.
    env.set_online(LAPTOP, false);
    let tree = env.tree();
    tree.tick();
    let open: Vec<String> = tree
        .open_children()
        .into_iter()
        .map(|(key, _)| key.name)
        .collect();
    assert_eq!(open, ["Music"]);
    assert_eq!(env.link.leases(), 1);
    // After its last handle it goes too, at the next round.
    tree.close(handle).unwrap();
    tree.tick();
    assert!(tree.open_children().is_empty());
    assert_eq!(env.link.leases(), 0);
}

#[test]
fn drive_task_devices_room_readonly() {
    let env = Env::new(&[Spec {
        id: "d1",
        name: "Laptop",
        room: Some(("r1", "Team")),
        writable: true,
        shares: &["Docs"],
    }]);
    let key = "room~r1~d1";
    std::fs::write(env.share_file(key, "Docs/a.txt"), b"x").unwrap();
    env.publish();
    let tree = env.tree();
    assert_eq!(names(tree.list_dir("/").unwrap()), ["Räume"]);
    assert_eq!(names(tree.list_dir("/Räume").unwrap()), ["Team"]);
    assert_eq!(names(tree.list_dir("/Räume/Team").unwrap()), ["Laptop"]);
    let handle = tree
        .open_file("/Räume/Team/Laptop/Docs/a.txt", open_existing())
        .unwrap();
    tree.close(handle).unwrap();
    // A room device is read-only even when the device itself would allow it.
    let error = tree
        .open_file(
            "/Räume/Team/Laptop/Docs/a.txt",
            OpenFileOptions {
                writable: true,
                disposition: OpenDisposition::OpenExisting,
            },
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(
        tree.mkdir("/Räume/Team/Laptop/Docs/new")
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
}

#[test]
fn drive_task_devices_child_mode_rules() {
    let env = Env::new(&[
        direct("a", "Laptop", &["Docs"]),
        Spec {
            id: "b",
            name: "Server",
            room: None,
            writable: false,
            shares: &["Docs"],
        },
        Spec {
            id: "d1",
            name: "Raum",
            room: Some(("r1", "Team")),
            writable: true,
            shares: &["Docs"],
        },
    ]);
    env.publish();
    let tree = env.tree();
    for path in ["/Laptop/Docs", "/Server/Docs", "/Räume/Team/Raum/Docs"] {
        tree.list_dir(path).unwrap();
    }
    let mode = |name: &str| {
        tree.open_children()
            .into_iter()
            .find(|(key, _)| key.key.ends_with(name))
            .map(|(_, child)| child.mode)
            .expect("open child")
    };
    // Direct device that allows writing: read-write.
    assert_eq!(mode("~a"), MountMode::ReadWrite);
    // Direct device that refuses writing: read-only.
    assert_eq!(mode("~b"), MountMode::ReadOnly);
    // A room device: always read-only.
    assert_eq!(mode("~d1"), MountMode::ReadOnly);
    create_file(tree, "/Laptop/Docs/new.txt", b"data").unwrap();
    assert_eq!(
        std::fs::read(env.share_file(LAPTOP, "Docs/new.txt")).unwrap(),
        b"data"
    );
    let error = tree
        .open_file("/Server/Docs/new.txt", create_options())
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
}

#[test]
fn drive_task_devices_cross_export_move_exdev() {
    let env = Env::new(&[
        direct("a", "Laptop", &["Docs", "Music"]),
        direct("b", "Büro", &["Docs"]),
    ]);
    env.publish();
    let tree = env.tree();
    create_file(tree, "/Laptop/Docs/a.txt", b"a").unwrap();
    // Inside one shared folder a move works.
    tree.rename("/Laptop/Docs/a.txt", "/Laptop/Docs/b.txt", false)
        .unwrap();
    assert!(env.share_file(LAPTOP, "Docs/b.txt").exists());
    // Between shared folders and between devices it is EXDEV.
    for (from, to) in [
        ("/Laptop/Docs/b.txt", "/Laptop/Music/b.txt"),
        ("/Laptop/Docs/b.txt", "/Büro/Docs/b.txt"),
    ] {
        let error = tree.rename(from, to, false).unwrap_err();
        assert_eq!(
            error.kind(),
            io::ErrorKind::CrossesDevices,
            "{from} -> {to}"
        );
    }
    assert!(env.share_file(LAPTOP, "Docs/b.txt").exists());
    assert!(!env.share_file(LAPTOP, "Music/b.txt").exists());
}

#[test]
fn drive_task_devices_virtual_layers_read_only() {
    let env = Env::new(&[
        direct("a", "Laptop", &["Docs", "Verbindungen/Srv"]),
        Spec {
            id: "d1",
            name: "Raum",
            room: Some(("r1", "Team")),
            writable: true,
            shares: &["Docs"],
        },
    ]);
    env.publish();
    let tree = env.tree();
    let denied = io::ErrorKind::PermissionDenied;
    let virtual_dirs = [
        "/",
        "/Laptop",
        "/Laptop/Verbindungen",
        "/Räume",
        "/Räume/Team",
        "/Räume/Team/Raum",
    ];
    for dir in virtual_dirs {
        let meta = tree.stat(dir).unwrap();
        assert!(meta.is_dir && meta.size == 0 && !meta.is_symlink, "{dir}");
        let child = format!("{}/New", dir.trim_end_matches('/'));
        assert_eq!(tree.mkdir(&child).unwrap_err().kind(), denied, "{child}");
        assert_eq!(
            tree.open_file(&child, create_options()).unwrap_err().kind(),
            denied,
            "{child}"
        );
        if dir != "/" {
            assert_eq!(tree.delete(dir, true).unwrap_err().kind(), denied, "{dir}");
            assert_eq!(
                tree.begin_delete(dir, true).unwrap_err().kind(),
                denied,
                "{dir}"
            );
            assert_eq!(
                tree.rename(dir, "/Other", false).unwrap_err().kind(),
                denied,
                "{dir}"
            );
            assert_eq!(
                tree.open_file(
                    dir,
                    OpenFileOptions {
                        writable: true,
                        disposition: OpenDisposition::OpenExisting,
                    }
                )
                .unwrap_err()
                .kind(),
                denied,
                "{dir}"
            );
        }
    }
    // A move into a virtual folder is refused as well.
    create_file(tree, "/Laptop/Docs/a.txt", b"a").unwrap();
    for to in ["/Laptop/a.txt", "/a.txt"] {
        let error = tree.rename("/Laptop/Docs/a.txt", to, false).unwrap_err();
        assert_eq!(error.kind(), denied, "{to}");
    }
}

#[test]
fn devices_task_windows_dialect_matches_names_in_any_case() {
    let env = Env::with_dialect(&[direct("a", "Laptop", &["Docs"])], PathDialect::Windows);
    std::fs::write(env.share_file(LAPTOP, "Docs/a.txt"), b"win").unwrap();
    env.publish();
    let tree = env.tree();
    assert!(!tree.volume_facts().case_sensitive);
    assert_eq!(names(tree.list_dir("\\").unwrap()), ["Laptop"]);
    assert_eq!(names(tree.list_dir("\\LAPTOP").unwrap()), ["Docs"]);
    assert!(tree.stat("\\laptop\\DOCS").unwrap().is_dir);
    let handle = tree
        .open_file("\\laptop\\docs\\a.txt", open_existing())
        .unwrap();
    let mut buffer = [0u8; 8];
    assert_eq!(tree.read(handle, 0, &mut buffer).unwrap(), 3);
    tree.close(handle).unwrap();
    assert_eq!(
        tree.mkdir("\\Laptop\\New").unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
}
