//! The GTK bookmarks file (`$XDG_CONFIG_HOME/gtk-3.0/bookmarks`, read by GTK
//! 3 and 4, Nautilus, Nemo and Thunar; NR N7): one line
//! `file://<percent-encoded path> <label>` per bookmark.
//!
//! A line carries no identity, so the path is the identity: removal drops
//! every line with the URI of the folder, whatever label the user gave it in
//! the file manager. Every other line stays byte for byte in its place.
use std::fs;
use std::io;
use std::path::Path;

use super::bookmarks::{uri_of, write_atomic, BookmarkEntry};
use super::mount_point::folder_name;

/// Adds `file://<folder> <label>` at the end, unless a line with the same
/// URI exists.
pub(super) fn add_in(file: &Path, entry: &BookmarkEntry) -> io::Result<()> {
    let uri = uri_of(&entry.folder);
    let mut content = read(file)?;
    if lines(&content).any(|line| uri_field(line) == uri.as_bytes()) {
        return Ok(());
    }
    if !content.is_empty() && !content.ends_with(b"\n") {
        content.push(b'\n');
    }
    content.extend_from_slice(uri.as_bytes());
    content.push(b' ');
    content.extend_from_slice(folder_name(&entry.label).as_bytes());
    content.push(b'\n');
    write_atomic(file, &content)
}

/// Removes every line whose URI is the one of `folder`. Without such a line
/// the file is not written.
pub(super) fn remove_in(file: &Path, folder: &Path) -> io::Result<()> {
    let uri = uri_of(folder);
    let content = read(file)?;
    let mut kept = Vec::with_capacity(content.len());
    let mut removed = false;
    for line in content.split_inclusive(|&byte| byte == b'\n') {
        if uri_field(line) == uri.as_bytes() {
            removed = true;
        } else {
            kept.extend_from_slice(line);
        }
    }
    if !removed {
        return Ok(());
    }
    if !kept.is_empty() && !kept.ends_with(b"\n") {
        kept.push(b'\n');
    }
    write_atomic(file, &kept)
}

fn read(file: &Path) -> io::Result<Vec<u8>> {
    match fs::read(file) {
        Ok(content) => Ok(content),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn lines(content: &[u8]) -> impl Iterator<Item = &[u8]> {
    content.split(|&byte| byte == b'\n')
}

/// The text before the first blank of a line, without a line break.
fn uri_field(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    match line.iter().position(|&byte| byte == b' ') {
        Some(end) => &line[..end],
        None => line,
    }
}
