//! Unix-only check of the link guard of `extract_all`: it needs a real
//! symbolic link, which only the OS adapter may create.
use super::extract_replace::test_support::{temp_base, write_zip};
use crate::zipfs::extract_all;
use std::os::unix::fs::symlink;

#[test]
fn zipfs_extract_does_not_follow_an_existing_link_in_the_target() {
    let base = temp_base("followlink");
    let outside = base.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("victim.txt"), b"untouched").unwrap();
    let out = base.join("out");
    std::fs::create_dir_all(&out).unwrap();
    // A folder link and a file link in the destination, both pointing out.
    symlink(&outside, out.join("dir")).unwrap();
    symlink(outside.join("victim.txt"), out.join("file.txt")).unwrap();
    let zip_path = base.join("arc.zip");
    write_zip(
        &zip_path,
        &[
            ("dir/victim.txt", b"overwritten"),
            ("dir/new.txt", b"planted"),
            ("file.txt", b"overwritten"),
            ("fine.txt", b"ok"),
        ],
        &[],
    );
    let zp = zip_path.to_string_lossy().replace('\\', "/");

    assert_eq!(extract_all(&zp, &out).unwrap(), 1);
    assert_eq!(
        std::fs::read(outside.join("victim.txt")).unwrap(),
        b"untouched"
    );
    assert!(!outside.join("new.txt").exists());
    assert_eq!(std::fs::read(out.join("fine.txt")).unwrap(), b"ok");

    // A destination folder that is itself a link is still used.
    let linked_dest = base.join("dest_link");
    symlink(&outside, &linked_dest).unwrap();
    let plain_zip = base.join("plain.zip");
    write_zip(&plain_zip, &[("fine.txt", b"ok")], &[]);
    let plain = plain_zip.to_string_lossy().replace('\\', "/");
    assert_eq!(extract_all(&plain, &linked_dest).unwrap(), 1);
    assert_eq!(std::fs::read(outside.join("fine.txt")).unwrap(), b"ok");

    let _ = std::fs::remove_dir_all(&base);
}
