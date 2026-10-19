//! The KDE places file (`$XDG_DATA_HOME/user-places.xbel`, read by Dolphin
//! and the KDE file dialogs; NR N7): XBEL with one `bookmark` element per
//! place.
//!
//! The entry of a drive carries the owner `urn:smart-explorer:places` with
//! the mount id; that is how se finds its own elements again. Every other
//! element, whatever its `href`, is never touched. The file is parsed with
//! `quick_xml::Reader` only to find the byte ranges of the elements; the
//! edit replaces exactly those ranges, so all other bytes (indentation, text,
//! comments) stay as they are. A file that is not well-formed XML is not
//! changed.
//!
//! Assumption (unchecked, no Dolphin on the test runner): KDE accepts any
//! unique string as `ID` and `folder-remote` is an existing icon. If not,
//! Dolphin shows the entry without an icon or assigns its own ID; the
//! identity of se stays the owner `urn:smart-explorer:places`.
use std::fs;
use std::io;
use std::path::Path;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::bookmarks::{uri_of, write_atomic, BookmarkEntry};
use super::mount_point::folder_name;

/// Owner of the metadata element that marks an entry as se's.
const OWNER: &str = "urn:smart-explorer:places";

/// A file that does not exist yet.
const TEMPLATE: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
    "<!DOCTYPE xbel>\n",
    "<xbel xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\" ",
    "xmlns:kdepriv=\"http://www.kde.org/kdepriv\" ",
    "xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\">\n",
    "</xbel>\n"
);

/// One `bookmark` element at the top of the tree: its byte range, its
/// `href`, whether it is se's, and where the whitespace-only text node in
/// front of it starts.
#[derive(Debug)]
struct Found {
    start: usize,
    end: usize,
    href: String,
    ours: bool,
    lead: Option<usize>,
}

struct Scan {
    found: Vec<Found>,
    /// Offset of `</xbel>`.
    close: usize,
}

/// Adds the entry of the drive before `</xbel>`; an element of se with the
/// same `href` is replaced (label and ID updated).
pub(super) fn add_in(file: &Path, entry: &BookmarkEntry) -> io::Result<()> {
    let (text, exists) = match read(file)? {
        Some(text) => (text, true),
        None => (TEMPLATE.to_string(), false),
    };
    let scan = scan(&text).map_err(invalid_data)?;
    let uri = uri_of(&entry.folder);
    let block = entry_xml(entry);
    let mut changed = text.clone();
    match scan
        .found
        .iter()
        .find(|found| found.ours && found.href == uri)
    {
        Some(found) => changed.replace_range(found.start..found.end, &block),
        None => {
            let separator = if text[..scan.close].is_empty() || text[..scan.close].ends_with('\n') {
                ""
            } else {
                "\n"
            };
            changed.insert_str(scan.close, &format!("{separator} {block}\n"));
        }
    }
    if exists && changed == text {
        return Ok(());
    }
    write_atomic(file, changed.as_bytes())
}

/// Removes every element of se whose `href` is the URI of `folder`, with the
/// whitespace-only text node in front of it. Without one the file is not
/// written.
pub(super) fn remove_in(file: &Path, folder: &Path) -> io::Result<()> {
    let Some(text) = read(file)? else {
        return Ok(());
    };
    let scan = scan(&text).map_err(invalid_data)?;
    let uri = uri_of(folder);
    let mut changed = text.clone();
    let mut removed = false;
    for found in scan
        .found
        .iter()
        .rev()
        .filter(|found| found.ours && found.href == uri)
    {
        changed.replace_range(found.lead.unwrap_or(found.start)..found.end, "");
        removed = true;
    }
    if !removed {
        return Ok(());
    }
    write_atomic(file, changed.as_bytes())
}

fn read(file: &Path) -> io::Result<Option<String>> {
    match fs::read(file) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| invalid_data("user-places.xbel ist kein UTF-8".into())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Finds the `bookmark` elements below the root and the end of the root.
fn scan(text: &str) -> Result<Scan, String> {
    let mut reader = Reader::from_str(text);
    let mut found = Vec::new();
    let mut open: Option<(Found, usize)> = None;
    let mut depth = 0usize;
    let mut close = None;
    let mut last_text: Option<usize> = None;
    loop {
        let before = reader.buffer_position() as usize;
        let event = reader.read_event().map_err(|error| error.to_string())?;
        let after = reader.buffer_position() as usize;
        let was_text = matches!(event, Event::Text(_));
        match event {
            Event::Start(element) => {
                depth += 1;
                if let Some((current, _)) = open.as_mut() {
                    current.ours |= marks_ours(&element)?;
                } else if element.name().as_ref() == b"bookmark" {
                    let begun = begin(&element, before, after, text, last_text)?;
                    open = Some((begun, depth));
                }
            }
            Event::Empty(element) => {
                if let Some((current, _)) = open.as_mut() {
                    current.ours |= marks_ours(&element)?;
                } else if element.name().as_ref() == b"bookmark" {
                    found.push(begin(&element, before, after, text, last_text)?);
                }
            }
            Event::End(element) => {
                if open.as_ref().is_some_and(|(_, at)| *at == depth) {
                    if let Some((mut done, _)) = open.take() {
                        done.end = after;
                        found.push(done);
                    }
                } else if depth == 1 && element.name().as_ref() == b"xbel" {
                    close = Some(before);
                }
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| "unerwartetes Ende-Tag".to_string())?;
            }
            Event::Eof => break,
            _ => {}
        }
        last_text = was_text.then_some(before);
    }
    if depth != 0 || open.is_some() {
        return Err("ein Element ist nicht abgeschlossen".into());
    }
    let close = close.ok_or_else(|| "kein xbel-Wurzelelement".to_string())?;
    Ok(Scan { found, close })
}

fn begin(
    element: &BytesStart<'_>,
    start: usize,
    end: usize,
    text: &str,
    last_text: Option<usize>,
) -> Result<Found, String> {
    let lead = last_text.filter(|&from| {
        from < start
            && text[from..start]
                .bytes()
                .all(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
    });
    Ok(Found {
        start,
        end,
        href: attribute(element, "href")?.unwrap_or_default(),
        ours: false,
        lead,
    })
}

/// `<metadata owner="urn:smart-explorer:places">`.
fn marks_ours(element: &BytesStart<'_>) -> Result<bool, String> {
    Ok(element.name().as_ref() == b"metadata"
        && attribute(element, "owner")?.is_some_and(|owner| owner == OWNER))
}

fn attribute(element: &BytesStart<'_>, name: &str) -> Result<Option<String>, String> {
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|error| error.to_string())?;
        if attribute.key.as_ref() == name.as_bytes() {
            let value = attribute
                .unescape_value()
                .map_err(|error| error.to_string())?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}

/// The element of a drive without its first indent (the caller places it);
/// one blank per level, as KDE writes it.
fn entry_xml(entry: &BookmarkEntry) -> String {
    let uri = escape(&uri_of(&entry.folder));
    let title = escape(&title_of(&entry.label));
    let id = escape(&entry.id);
    format!(
        "<bookmark href=\"{uri}\">\n\
         \x20 <title>{title}</title>\n\
         \x20 <info>\n\
         \x20  <metadata owner=\"http://freedesktop.org\">\n\
         \x20   <bookmark:icon name=\"folder-remote\"/>\n\
         \x20  </metadata>\n\
         \x20  <metadata owner=\"http://www.kde.org\">\n\
         \x20   <ID>se-{id}</ID>\n\
         \x20   <isSystemItem>false</isSystemItem>\n\
         \x20  </metadata>\n\
         \x20  <metadata owner=\"{OWNER}\">\n\
         \x20   <id>{id}</id>\n\
         \x20  </metadata>\n\
         \x20 </info>\n\
         \x20</bookmark>"
    )
}

/// The label without control characters; an empty label falls back to the
/// folder name rules.
fn title_of(label: &str) -> String {
    let title: String = label
        .trim()
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    if title.trim().is_empty() {
        folder_name(label)
    } else {
        title
    }
}

fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn invalid_data(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
