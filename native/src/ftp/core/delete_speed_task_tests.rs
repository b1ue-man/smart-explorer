//! Delete-speed task tests for FTP folder deletion without a server: DELE
//! runs in leaf order and sends nothing after a failure, and the
//! names-and-types listing keeps links out of the walk (MLSD facts and LIST
//! rows) and fails on any entry it cannot classify. The wire run is covered
//! by the remote suite.
use super::tree_remove::{entries, remove_in_order};
use crate::vfs::DeleteTarget;
use std::io;

fn leaves(count: usize) -> Vec<DeleteTarget> {
    (0..count)
        .map(|index| DeleteTarget {
            path: format!("/tree/leaf-{index}"),
            id: None,
            is_dir: false,
            is_symlink: false,
        })
        .collect()
}

fn lines(rows: &[&str]) -> Vec<String> {
    rows.iter().map(|row| row.to_string()).collect()
}

#[test]
fn delete_speed_task_ftp_dele_runs_in_order_and_stops_after_a_failure() {
    let leaves = leaves(5);
    let mut sent = Vec::new();
    let results = remove_in_order(&leaves, |leaf| {
        sent.push(leaf.path.clone());
        if leaf.path.ends_with("-2") {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "550 denied",
            ))
        } else {
            Ok(())
        }
    });
    assert_eq!(sent, ["/tree/leaf-0", "/tree/leaf-1", "/tree/leaf-2"]);
    assert_eq!(results.len(), 5);
    assert!(results[0].is_ok() && results[1].is_ok());
    for result in &results[2..] {
        let error = result.as_ref().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "550 denied");
    }
}

#[test]
fn delete_speed_task_ftp_types_listing_keeps_links_as_leaves() {
    let mlsd = entries(
        lines(&[
            "type=cdir; .",
            "type=pdir; ..",
            "type=dir; sub",
            "type=file;size=3; a.txt",
            "type=OS.unix=symlink; link",
            "type=OS.unix=slink:/elsewhere; slink",
            "type=link; plain-link",
        ]),
        true,
    )
    .unwrap();
    let kinds: Vec<(&str, bool, bool)> = mlsd
        .iter()
        .map(|entry| (entry.name.as_str(), entry.is_dir, entry.is_symlink))
        .collect();
    assert_eq!(
        kinds,
        [
            ("sub", true, false),
            ("a.txt", false, false),
            ("link", false, true),
            ("slink", false, true),
            ("plain-link", false, true),
        ]
    );

    let list = entries(
        lines(&[
            "total 3",
            "drwxr-xr-x 1 0 0 0 Jan 1 2020 .",
            "drwxr-xr-x 1 0 0 0 Jan 1 2020 sub",
            "-rw-r--r-- 1 0 0 3 Jan 1 2020 a.txt",
            "lrwxrwxrwx 1 0 0 3 Jan 1 2020 link -> sub",
        ]),
        false,
    )
    .unwrap();
    let kinds: Vec<(&str, bool, bool)> = list
        .iter()
        .map(|entry| (entry.name.as_str(), entry.is_dir, entry.is_symlink))
        .collect();
    assert_eq!(
        kinds,
        [
            ("sub", true, false),
            ("a.txt", false, false),
            ("link", false, true),
        ]
    );
}

#[test]
fn delete_speed_task_ftp_types_listing_fails_on_unknown_entries() {
    // A FIFO row matches no LIST format.
    assert!(entries(lines(&["prw-r--r-- 1 0 0 0 Jan 1 2020 fifo"]), false).is_err());
    let repeated = ["type=file;size=1; a", "type=file;size=1; a"];
    assert!(entries(lines(&repeated), true).is_err());
    assert!(entries(lines(&["type=file;size=1; back\\slash"]), true).is_err());
    assert!(entries(lines(&["no-facts-row"]), true).is_err());
}
