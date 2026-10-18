//! Freedesktop thumbnails of vault drives (K45, S2-V7, VLT-M12; Linux).
//!
//! GNOME and every program that follows the freedesktop thumbnail
//! specification keep thumbnails in `$XDG_CACHE_HOME/thumbnails` (default
//! `~/.cache/thumbnails`): `normal/`, `large/`, `x-large/`, `xx-large/` and
//! `fail/<program>/`, one PNG per file, named after the MD5 of its URI, with
//! the URI of the original in the text chunk `Thumb::URI` (a required key of
//! the specification). The pictures of a vault drive are cleartext; when the
//! drive ends or its vault locks, every thumbnail whose `Thumb::URI` names a
//! file inside the drive's folder is removed. The URI is read instead of the
//! MD5 name computed, so every escaping of the URI counts.
//!
//! Nothing here follows a link: links in the cache are skipped, files are
//! opened with `O_NOFOLLOW`, folders are entered only when they are plain
//! folders, and only plain PNG files are unlinked. Reading stops after a
//! bounded number of chunks and bytes per file.
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

const SIZES: [&str; 4] = ["normal", "large", "x-large", "xx-large"];
const FAIL: &str = "fail";
const KEY: &[u8] = b"Thumb::URI";
const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
/// Text chunks above this size are skipped; a URI is far shorter.
const MAX_TEXT: u32 = 64 * 1024;
/// Chunks read before one file counts as without URI.
const MAX_CHUNKS: usize = 256;

/// `$XDG_CACHE_HOME/thumbnails`, else `~/.cache/thumbnails`.
pub(super) fn cache_root() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|cache| cache.is_absolute())
        .or_else(|| crate::support_dirs::home_dir().map(|home| home.join(".cache")))
        .map(|cache| cache.join("thumbnails"))
}

/// Removes every thumbnail below `root` whose `Thumb::URI` names `place`
/// or a file inside it; returns how many it removed.
pub(super) fn remove_for_place(root: &Path, place: &Path) -> usize {
    let places = spellings(place);
    let mut removed = 0;
    for size in SIZES {
        removed += clean_folder(&root.join(size), &places);
    }
    let fail = root.join(FAIL);
    if plain_dir(&fail) {
        if let Ok(entries) = fs::read_dir(&fail) {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    removed += clean_folder(&entry.path(), &places);
                }
            }
        }
    }
    removed
}

/// The folder as configured and as its resolved parent names it.
fn spellings(place: &Path) -> Vec<PathBuf> {
    let mut places = vec![place.to_path_buf()];
    let resolved = place
        .parent()
        .and_then(|parent| fs::canonicalize(parent).ok())
        .zip(place.file_name())
        .map(|(parent, name)| parent.join(name));
    if let Some(resolved) = resolved.filter(|resolved| resolved != place) {
        places.push(resolved);
    }
    places
}

fn plain_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir())
}

fn clean_folder(folder: &Path, places: &[PathBuf]) -> usize {
    if !plain_dir(folder) {
        return 0;
    }
    let Ok(entries) = fs::read_dir(folder) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.extension().is_some_and(|extension| extension == "png") {
            continue;
        }
        // `file_type` of a directory entry does not follow links.
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let Some(uri) = thumbnail_uri(&path) else {
            continue;
        };
        let inside = file_uri_path(&uri)
            .is_some_and(|named| places.iter().any(|place| named.starts_with(place)));
        // `remove_file` unlinks the entry itself, never a link target.
        if inside && fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// The `Thumb::URI` of a PNG file; `None` for anything else.
fn thumbnail_uri(path: &Path) -> Option<Vec<u8>> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut signature = [0u8; 8];
    file.read_exact(&mut signature).ok()?;
    if signature != SIGNATURE {
        return None;
    }
    for _ in 0..MAX_CHUNKS {
        let mut head = [0u8; 8];
        file.read_exact(&mut head).ok()?;
        let length = u32::from_be_bytes([head[0], head[1], head[2], head[3]]);
        let kind = [head[4], head[5], head[6], head[7]];
        if &kind == b"IEND" {
            return None;
        }
        let text = matches!(&kind, b"tEXt" | b"zTXt" | b"iTXt");
        if text && length <= MAX_TEXT {
            let mut data = vec![0u8; length as usize];
            file.read_exact(&mut data).ok()?;
            if let Some(uri) = text_value(&kind, &data) {
                return Some(uri);
            }
            file.seek(SeekFrom::Current(4)).ok()?;
        } else {
            file.seek(SeekFrom::Current(i64::from(length) + 4)).ok()?;
        }
    }
    None
}

/// The value of a text chunk whose keyword is `Thumb::URI` (PNG 1.2:
/// `tEXt` plain, `zTXt` zlib, `iTXt` plain or zlib).
fn text_value(kind: &[u8; 4], data: &[u8]) -> Option<Vec<u8>> {
    let end = data.iter().position(|byte| *byte == 0)?;
    if &data[..end] != KEY {
        return None;
    }
    let rest = &data[end + 1..];
    match kind {
        b"tEXt" => Some(rest.to_vec()),
        b"zTXt" => match rest.split_first()? {
            (&0, compressed) => inflate(compressed),
            _ => None,
        },
        b"iTXt" => {
            let (&flag, rest) = rest.split_first()?;
            let (&method, rest) = rest.split_first()?;
            let language = rest.iter().position(|byte| *byte == 0)?;
            let rest = &rest[language + 1..];
            let translated = rest.iter().position(|byte| *byte == 0)?;
            let text = &rest[translated + 1..];
            match (flag, method) {
                (0, _) => Some(text.to_vec()),
                (1, 0) => inflate(text),
                _ => None,
            }
        }
        _ => None,
    }
}

fn inflate(compressed: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(compressed)
        .take(u64::from(MAX_TEXT))
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

/// The local path of a `file:` URI (`file:///p` or `file://localhost/p`),
/// percent-decoded; query and fragment end it.
pub(super) fn file_uri_path(uri: &[u8]) -> Option<PathBuf> {
    let rest = uri.strip_prefix(b"file://")?;
    let rest = rest.strip_prefix(b"localhost").unwrap_or(rest);
    if rest.first() != Some(&b'/') {
        return None;
    }
    let mut bytes = Vec::with_capacity(rest.len());
    let mut index = 0;
    while index < rest.len() {
        match rest[index] {
            b'%' => {
                let hex = std::str::from_utf8(rest.get(index + 1..index + 3)?).ok()?;
                bytes.push(u8::from_str_radix(hex, 16).ok()?);
                index += 3;
            }
            b'?' | b'#' => break,
            byte => {
                bytes.push(byte);
                index += 1;
            }
        }
    }
    Some(PathBuf::from(OsString::from_vec(bytes)))
}

#[cfg(test)]
#[path = "linux_thumbnails_tests.rs"]
mod tests;
