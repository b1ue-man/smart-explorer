use super::*;

#[test]
fn removable_task_fat_family_numbers_follow_the_entry() {
    // NTFS and ReFS answer with FILE_SUPPORTS_OPEN_BY_FILE_ID and keep
    // their numbers; the FAT family never does, whatever flags it reports.
    assert!(!entry_bound("NTFS", OPEN_BY_FILE_ID | 0x2));
    assert!(!entry_bound("ReFS", OPEN_BY_FILE_ID));
    for name in ["FAT", "FAT32", "fat32", "exFAT", "EXFAT", "FAT16", "FAT12"] {
        assert!(entry_bound(name, 0), "{name}");
        assert!(entry_bound(name, OPEN_BY_FILE_ID), "{name} with the flag");
    }
    // A filesystem that cannot open a file by its number has no lasting
    // number either.
    assert!(entry_bound("UDF", 0x2));
    assert!(entry_bound("", 0));
}

#[test]
fn removable_task_snapshot_keeps_the_number_on_the_system_volume() {
    // The test directory lies on the runner's NTFS volume: its snapshot
    // identity is the full one, unchanged.
    let path = std::env::temp_dir().join(format!(
        "se-entry-identity-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, b"x").unwrap();
    let file = File::open(&path).unwrap();
    let identity = super::super::file_identity(&file).unwrap();
    assert_ne!(identity.1, 0, "NTFS reports a file number");
    assert_eq!(snapshot_identity(&file, identity), identity);
    drop(file);
    std::fs::remove_file(&path).unwrap();
}
