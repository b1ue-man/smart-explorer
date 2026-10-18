//! The inode table of the FUSE host (DRV-M4): lookups are counted, numbers
//! are never reused, renames carry descendants along, and listing numbers
//! never create nodes.
use super::{join, split, InodeTable, ROOT_INO};

#[test]
fn fuse_inodes_count_lookups_and_never_reuse_numbers() {
    let table = InodeTable::new();
    assert_eq!(table.path(ROOT_INO).unwrap(), "/");
    assert_eq!(table.child_path(ROOT_INO, "a").unwrap(), "/a");
    let first = table.remember("/a").unwrap();
    assert_eq!(table.remember("/a").unwrap(), first);
    assert_eq!(table.known("/a"), Some(first));
    // One of two lookups forgotten: the node stays.
    table.forget(first, 1);
    assert_eq!(table.path(first).unwrap(), "/a");
    table.forget(first, 1);
    assert!(table.path(first).is_err());
    assert_eq!(table.known("/a"), None);
    // A new lookup of the same name gets a new number.
    let second = table.remember("/a").unwrap();
    assert_ne!(second, first);
    // The root is never forgotten.
    table.forget(ROOT_INO, 100);
    assert_eq!(table.path(ROOT_INO).unwrap(), "/");
    assert_eq!(table.len(), 2);
}

#[test]
fn fuse_inodes_rename_moves_descendants_and_unlinks_replaced() {
    let table = InodeTable::new();
    let directory = table.remember("/docs").unwrap();
    let file = table.remember("/docs/a.txt").unwrap();
    let deep = table.remember("/docs/sub/b.txt").unwrap();
    let sibling = table.remember("/docs-old/c.txt").unwrap();
    let target = table.remember("/archive").unwrap();
    table.renamed("/docs", "/archive");
    assert_eq!(table.path(directory).unwrap(), "/archive");
    assert_eq!(table.path(file).unwrap(), "/archive/a.txt");
    assert_eq!(table.path(deep).unwrap(), "/archive/sub/b.txt");
    // A name that only shares the prefix stays where it is.
    assert_eq!(table.path(sibling).unwrap(), "/docs-old/c.txt");
    // The replaced node lives on for its handles but resolves no path.
    assert!(table.path(target).is_err());
    assert_eq!(table.known("/archive"), Some(directory));
    assert_eq!(table.known("/docs/a.txt"), None);
    // Unlinking keeps the number until the kernel forgets it.
    table.unlinked("/archive/a.txt");
    assert!(table.path(file).is_err());
    assert_eq!(table.known("/archive/a.txt"), None);
    table.forget(file, 1);
    table.forget(target, 1);
    assert_eq!(table.len(), 4);
    // A rename onto itself changes nothing.
    table.renamed("/archive", "/archive");
    assert_eq!(table.path(directory).unwrap(), "/archive");
}

#[test]
fn fuse_inodes_listing_numbers_create_no_nodes() {
    let table = InodeTable::new();
    let known = table.remember("/known").unwrap();
    assert_eq!(table.listing_ino("/known"), known);
    let pseudo = table.listing_ino("/unknown");
    assert_ne!(pseudo & (1 << 63), 0);
    assert_eq!(pseudo, table.listing_ino("/unknown"));
    assert_ne!(pseudo, table.listing_ino("/other"));
    assert_eq!(table.len(), 2);
    assert_eq!(join("/", "a"), "/a");
    assert_eq!(join("/a", "b"), "/a/b");
    assert_eq!(split("/a"), Some(("/", "a")));
    assert_eq!(split("/a/b"), Some(("/a", "b")));
    assert_eq!(split("/"), None);
}
