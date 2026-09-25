use std::io;
use std::path::{Path, PathBuf};

use super::record::{numbered_name, plain_name, valid_id};
use super::store::{delete_in, list_in, move_to_trash_in, purge_older_than_in, restore_in};
use super::{is_excluded, TRASH_DIR_NAME};

const DAY_MS: i64 = 86_400_000;

fn volume() -> (tempfile::TempDir, Vec<PathBuf>) {
    let fixture = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(fixture.path()).unwrap();
    (fixture, vec![root])
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn android_task_apptrash_round_trip_restores_the_original() {
    let (_fixture, volumes) = volume();
    let original = volumes[0].join("docs").join("note.txt");
    write(&original, "hello");

    let entry = move_to_trash_in(&volumes, &original).unwrap();
    assert!(!original.exists());
    assert_eq!(entry.name, "note.txt");
    assert_eq!(entry.original, original);
    assert_eq!(entry.size, 5);
    assert!(!entry.is_dir);
    let trash = volumes[0].join(TRASH_DIR_NAME);
    assert!(trash.join(&entry.id).join("note.txt").is_file());
    assert!(trash.join(format!("{}.json", entry.id)).is_file());
    assert_eq!(list_in(&volumes).unwrap(), vec![entry.clone()]);

    assert_eq!(restore_in(&volumes, &entry.id).unwrap(), original);
    assert_eq!(std::fs::read_to_string(&original).unwrap(), "hello");
    assert!(list_in(&volumes).unwrap().is_empty());
    assert!(!trash.join(&entry.id).exists());
    assert!(!trash.join(format!("{}.json", entry.id)).exists());
}

#[test]
fn android_task_apptrash_restore_never_overwrites_an_occupied_name() {
    let (_fixture, volumes) = volume();
    let original = volumes[0].join("photo.jpg");
    write(&original, "old");
    let entry = move_to_trash_in(&volumes, &original).unwrap();
    write(&original, "new");

    let restored = restore_in(&volumes, &entry.id).unwrap();
    assert_eq!(restored, volumes[0].join("photo (2).jpg"));
    assert_eq!(std::fs::read_to_string(&original).unwrap(), "new");
    assert_eq!(std::fs::read_to_string(&restored).unwrap(), "old");
}

#[test]
fn android_task_apptrash_restore_recreates_a_removed_parent() {
    let (_fixture, volumes) = volume();
    let original = volumes[0].join("gone").join("deep").join("a.bin");
    write(&original, "x");
    let entry = move_to_trash_in(&volumes, &original).unwrap();
    std::fs::remove_dir_all(volumes[0].join("gone")).unwrap();

    assert_eq!(restore_in(&volumes, &entry.id).unwrap(), original);
    assert!(original.is_file());
}

#[test]
fn android_task_apptrash_folders_report_their_size_and_delete_permanently() {
    let (_fixture, volumes) = volume();
    let folder = volumes[0].join("album");
    write(&folder.join("one.txt"), "123");
    write(&folder.join("inner").join("two.txt"), "4567");

    let entry = move_to_trash_in(&volumes, &folder).unwrap();
    assert!(entry.is_dir);
    assert_eq!(entry.size, 7);
    assert!(!folder.exists());

    delete_in(&volumes, &entry.id).unwrap();
    assert!(list_in(&volumes).unwrap().is_empty());
    assert!(!volumes[0].join(TRASH_DIR_NAME).join(&entry.id).exists());
    let error = delete_in(&volumes, &entry.id).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}

#[test]
fn android_task_apptrash_purge_removes_only_old_entries() {
    let (_fixture, volumes) = volume();
    let first = volumes[0].join("a.txt");
    let second = volumes[0].join("b.txt");
    write(&first, "a");
    write(&second, "b");
    let entry = move_to_trash_in(&volumes, &first).unwrap();
    move_to_trash_in(&volumes, &second).unwrap();

    assert_eq!(
        purge_older_than_in(&volumes, 30, entry.deleted_ms + DAY_MS).unwrap(),
        0
    );
    assert_eq!(list_in(&volumes).unwrap().len(), 2);
    assert_eq!(
        purge_older_than_in(&volumes, 30, entry.deleted_ms + 31 * DAY_MS).unwrap(),
        2
    );
    assert!(list_in(&volumes).unwrap().is_empty());
}

#[test]
fn android_task_apptrash_refuses_outside_volumes_and_the_trash_itself() {
    let (_fixture, volumes) = volume();
    let (_other_fixture, other) = volume();
    let outside = other[0].join("x.txt");
    write(&outside, "x");
    let error = move_to_trash_in(&volumes, &outside).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    assert!(outside.is_file());

    let inside = volumes[0].join("y.txt");
    write(&inside, "y");
    let entry = move_to_trash_in(&volumes, &inside).unwrap();
    let payload = volumes[0]
        .join(TRASH_DIR_NAME)
        .join(&entry.id)
        .join("y.txt");
    let error = move_to_trash_in(&volumes, &payload).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert!(payload.is_file());

    let unmanaged = volumes[0].join("z.txt");
    write(&unmanaged, "z");
    let error = move_to_trash_in(&[], &unmanaged).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    assert!(unmanaged.is_file());
}

#[test]
fn android_task_apptrash_ignores_planted_records() {
    let (_fixture, volumes) = volume();
    let (_other_fixture, other) = volume();
    let trash = volumes[0].join(TRASH_DIR_NAME);
    let id = "1-00000000000000aa";
    write(&trash.join(id).join("evil.txt"), "payload");
    let planted = format!(
        "{{\"version\":1,\"id\":\"{id}\",\"name\":\"evil.txt\",\"original\":{},\
         \"deletedMs\":1,\"size\":7,\"isDir\":false}}",
        serde_json::to_string(&other[0].join("evil.txt")).unwrap()
    );
    write(&trash.join(format!("{id}.json")), &planted);

    assert!(list_in(&volumes).unwrap().is_empty());
    let error = restore_in(&volumes, id).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(!other[0].join("evil.txt").exists());
    let error = restore_in(&volumes, "../escape").unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn android_task_apptrash_names_ids_and_exclusion_rules() {
    assert_eq!(numbered_name("photo.jpg", false, 1), "photo.jpg");
    assert_eq!(numbered_name("photo.jpg", false, 2), "photo (2).jpg");
    assert_eq!(numbered_name(".profile", false, 3), ".profile (3)");
    assert_eq!(numbered_name("v1.2", true, 2), "v1.2 (2)");

    assert!(plain_name("a.txt"));
    for name in ["", ".", "..", "a/b", "/a"] {
        assert!(!plain_name(name), "{name:?}");
    }
    assert!(valid_id("18f2c-00ff"));
    let too_long = "a".repeat(65);
    for id in ["", "../x", "ABC", "a b", too_long.as_str()] {
        assert!(!valid_id(id), "{id:?}");
    }

    assert!(is_excluded(TRASH_DIR_NAME, || true));
    assert!(!is_excluded(TRASH_DIR_NAME, || false));
    assert!(!is_excluded("Papierkorb", || true));
}
