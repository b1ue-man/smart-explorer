//! The wire formats of the file clipboard on X11 (U34-8): `text/uri-list`
//! (RFC 2483, `file://` URIs with RFC 3986 percent encoding), GNOME's
//! `x-special/gnome-copied-files` (`copy` or `cut`, then the URIs) and the
//! marker text of the surface. The same formats as the Wayland backend's
//! `clipboard_uri.rs`; pure functions.
use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use crate::app::desktop_surface_actions::clip_text;

pub(super) const URI_LIST: &str = "text/uri-list";
pub(super) const GNOME_COPIED: &str = "x-special/gnome-copied-files";
pub(super) const TEXT_UTF8: &str = "text/plain;charset=utf-8";
pub(super) const TEXT: &str = "text/plain";
pub(super) const UTF8_STRING: &str = "UTF8_STRING";

/// The targets a selection of files answers, best first.
pub(super) const FILE_TARGETS: [&str; 6] = [
    URI_LIST,
    GNOME_COPIED,
    UTF8_STRING,
    TEXT_UTF8,
    TEXT,
    "STRING",
];

/// The bytes for `target` of a selection of `files`; `None` for a target
/// that is not one of [`FILE_TARGETS`].
pub(super) fn bytes_for(files: &[PathBuf], cut: bool, target: &str) -> Option<Vec<u8>> {
    match target {
        URI_LIST => Some(uri_list(files).into_bytes()),
        GNOME_COPIED => Some(gnome_copied(files, cut).into_bytes()),
        // The marker text of the surface: a second se surface and programs
        // that take only text keep working.
        UTF8_STRING | TEXT_UTF8 | TEXT | "STRING" => Some(clip_text(files, cut).into_bytes()),
        _ => None,
    }
}

/// `file://` and the path with everything but `[A-Za-z0-9-._~/]` as `%XX`.
pub(super) fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            uri.push(char::from(*byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// `text/uri-list`: one URI per line, `\r\n` after each.
pub(super) fn uri_list(files: &[PathBuf]) -> String {
    files
        .iter()
        .map(|file| format!("{}\r\n", file_uri(file)))
        .collect()
}

/// `x-special/gnome-copied-files`: `copy` or `cut`, then the URIs, joined
/// by `\n`.
pub(super) fn gnome_copied(files: &[PathBuf], cut: bool) -> String {
    let mut lines = vec![if cut { "cut" } else { "copy" }.to_string()];
    lines.extend(files.iter().map(|file| file_uri(file)));
    lines.join("\n")
}

fn hex(byte: u8) -> Option<u8> {
    char::from(byte).to_digit(16).map(|digit| digit as u8)
}

/// Undoes the percent encoding; `None` for a `%` without two hex digits.
fn percent_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex(*bytes.get(index + 1)?)?;
            let low = hex(*bytes.get(index + 2)?)?;
            decoded.push(high * 16 + low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    Some(decoded)
}

/// The path of a `file://` URI without a host or with the host `localhost`;
/// `None` for another scheme, another host, a relative path or a NUL byte.
pub(super) fn parse_file_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.trim().strip_prefix("file://")?;
    let path = if rest.starts_with('/') {
        rest
    } else {
        let (host, path) = rest.split_at(rest.find('/')?);
        if !host.eq_ignore_ascii_case("localhost") {
            return None;
        }
        path
    };
    let bytes = percent_decode(path)?;
    if bytes.contains(&0) || bytes.first() != Some(&b'/') {
        return None;
    }
    Some(PathBuf::from(OsString::from_vec(bytes)))
}

/// The files of a selection body and whether they were cut. The GNOME type
/// starts with `copy` or `cut`; `text/uri-list` is always a copy and may
/// carry `#` comment lines. Lines that are no usable URI, or name a file that
/// `exists` rejects, are left out; no file at all gives `None`.
pub(super) fn parse_file_list(
    target: &str,
    body: &str,
    exists: impl Fn(&Path) -> bool,
) -> Option<(Vec<PathBuf>, bool)> {
    let mut lines = body.lines();
    let mut cut = false;
    if target == GNOME_COPIED {
        match lines.next().map(str::trim) {
            Some("cut") => cut = true,
            Some("copy") => {}
            // Without the action word the first line is already a URI.
            _ => lines = body.lines(),
        }
    }
    let files: Vec<PathBuf> = lines
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .filter_map(parse_file_uri)
        .filter(|path| exists(path))
        .collect();
    (!files.is_empty()).then_some((files, cut))
}
