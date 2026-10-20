//! DSK-M6 checks of the X11 file clipboard: the formats (pure,
//! `x11_clipboard_formats_roundtrip`) and, on Xvfb with two providers of this
//! process (two connections, two windows), `desktop_task_x11_clipboard_roundtrip_files`
//! and `desktop_task_x11_clipboard_too_large_refused` (suite stage
//! `linux-desktop: x11`, `SE_DESKTOP_TASK_X11=1`).
use std::path::PathBuf;

use super::super::clipboard_formats::{
    bytes_for, gnome_copied, parse_file_list, parse_file_uri, uri_list, FILE_TARGETS, GNOME_COPIED,
    URI_LIST,
};
use super::super::x11_task_env::{require_task, XServer};
use super::*;

fn paths(names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(|name| PathBuf::from(*name)).collect()
}

#[test]
fn x11_clipboard_formats_roundtrip() {
    let files = paths(&["/home/u/Desktop/a b.txt", "/tmp/ä%x.txt", "/data/plain"]);
    let list = uri_list(&files);
    assert_eq!(
        list,
        "file:///home/u/Desktop/a%20b.txt\r\nfile:///tmp/%C3%A4%25x.txt\r\nfile:///data/plain\r\n"
    );
    assert_eq!(
        gnome_copied(&files, true).lines().next(),
        Some("cut"),
        "the action word first"
    );
    assert_eq!(gnome_copied(&files, false).lines().next(), Some("copy"));
    assert!(!gnome_copied(&files, false).contains('\r'));
    let everything = |_: &std::path::Path| true;
    assert_eq!(
        parse_file_list(URI_LIST, &list, everything),
        Some((files.clone(), false)),
        "text/uri-list is always a copy"
    );
    assert_eq!(
        parse_file_list(GNOME_COPIED, &gnome_copied(&files, true), everything),
        Some((files.clone(), true))
    );
    // Comment lines, other schemes, other hosts and files that are gone.
    let mixed = "# comment\r\nhttp://example.org/a\r\nfile://other/host\r\n\
                 file://localhost/tmp/keep.txt\r\nfile:///tmp/gone.txt\r\n\r\n";
    let found = parse_file_list(URI_LIST, mixed, |path| path.ends_with("keep.txt"));
    assert_eq!(found, Some((paths(&["/tmp/keep.txt"]), false)));
    assert_eq!(parse_file_list(URI_LIST, mixed, |_| false), None);
    // The first line without an action word is already a URI.
    let bare = "file:///tmp/one.txt\nfile:///tmp/two.txt";
    assert_eq!(
        parse_file_list(GNOME_COPIED, bare, everything),
        Some((paths(&["/tmp/one.txt", "/tmp/two.txt"]), false))
    );
    assert_eq!(parse_file_uri("file:///a%2"), None);
    assert_eq!(parse_file_uri("file:///a%00b"), None, "no NUL byte");
    assert_eq!(parse_file_uri("file:relative"), None);
    // Every target of the answer has bytes; others do not.
    for target in FILE_TARGETS {
        assert!(bytes_for(&files, false, target).is_some(), "{target}");
    }
    assert!(bytes_for(&files, false, "image/png").is_none());
    let marker = String::from_utf8(bytes_for(&files, true, "text/plain").unwrap()).unwrap();
    assert!(marker.starts_with("x-se-desktop-cut\n/home/u/Desktop/a b.txt"));
}

fn existing(dir: &std::path::Path, names: &[&str]) -> Vec<PathBuf> {
    names
        .iter()
        .map(|name| {
            let path = dir.join(name);
            std::fs::write(&path, b"x").unwrap();
            path
        })
        .collect()
}

#[test]
#[ignore = "braucht Xvfb (Stufe linux-desktop: x11)"]
fn desktop_task_x11_clipboard_roundtrip_files() {
    require_task();
    let _x = XServer::start("320x200x24");
    let dir = tempfile::tempdir().unwrap();
    let files = existing(
        dir.path(),
        &["a.txt", "b ä%.txt", "Ordner mit Leerzeichen.txt"],
    );
    let writer = X11Clipboard::start().expect("provider one");
    let reader = X11Clipboard::start().expect("provider two");

    // Nothing was copied yet.
    assert_eq!(reader.read(), Ok(None));
    // Copy in one program, paste in another.
    writer.write(&files, false).expect("copy");
    assert_eq!(reader.read(), Ok(Some((files.clone(), false))));
    // The owner reads its own copy without a round trip.
    assert_eq!(writer.read(), Ok(Some((files.clone(), false))));
    // Cut is told as well.
    writer.write(&files[..2], true).expect("cut");
    assert_eq!(reader.read(), Ok(Some((files[..2].to_vec(), true))));
    // The other program takes the selection over: the first reads it back.
    reader.write(&files[2..], false).expect("copy elsewhere");
    assert_eq!(writer.read(), Ok(Some((files[2..].to_vec(), false))));
    // A copy of files that are gone is no file list any more.
    std::fs::remove_file(&files[2]).unwrap();
    assert_eq!(writer.read(), Ok(None));
}

#[test]
#[ignore = "braucht Xvfb (Stufe linux-desktop: x11)"]
fn desktop_task_x11_clipboard_too_large_refused() {
    require_task();
    let _x = XServer::start("320x200x24");
    let dir = tempfile::tempdir().unwrap();
    let small = existing(dir.path(), &["small.txt"]);
    let writer = X11Clipboard::start().expect("provider one");
    let reader = X11Clipboard::start().expect("provider two");
    writer.write(&small, false).expect("a small list");

    // More bytes than one request carries: no INCR, so refused with the text
    // of the contract, and the earlier selection stays.
    let limit = writer
        .shared
        .conn
        .maximum_request_bytes()
        .saturating_sub(REQUEST_SLACK);
    let name_len = 90;
    let count = limit / name_len + 1000;
    let many: Vec<PathBuf> = (0..count)
        .map(|number| PathBuf::from(format!("/tmp/{number:0>width$}", width = name_len)))
        .collect();
    assert_eq!(writer.write(&many, false), Err(TOO_MANY.to_string()));
    assert_eq!(reader.read(), Ok(Some((small.clone(), false))));
    // Within the limit it works again.
    writer.write(&small, true).expect("small again");
    assert_eq!(reader.read(), Ok(Some((small, true))));
}
