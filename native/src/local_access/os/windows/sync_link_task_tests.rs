use super::{kind, link_like, EntryKind, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT};

#[test]
fn sync_links_task_windows_cloud_data_tags_are_not_redirecting_links() {
    let file = FILE_ATTRIBUTE_REPARSE_POINT;
    let directory = file | FILE_ATTRIBUTE_DIRECTORY;
    // CLOUD through CLOUD_F are data reparse points, as is WOF.
    for tag in (0..=15).map(|index| 0x9000_001a | (index << 12)).chain([0x8000_0017]) {
        assert!(!link_like(file, tag), "data tag {tag:x}");
        assert!(!link_like(directory, tag));
        assert!(matches!(kind(file, tag), EntryKind::File));
        assert!(matches!(kind(directory, tag), EntryKind::Directory));
    }
    for tag in [0xa000_000c, 0xa000_0003, 0xa123_4567] {
        assert!(link_like(file, tag));
        assert!(link_like(directory, tag));
        assert!(matches!(kind(directory, tag), EntryKind::Link));
    }
    assert!(link_like(file, 0), "unknown reparse tag remains protected");
    assert!(!link_like(0, 0));
    assert!(!link_like(FILE_ATTRIBUTE_DIRECTORY, 0));
}
