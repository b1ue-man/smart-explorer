//! Entry-name check shared by browsing and both extraction paths.
//!
//! zip 2 refused every entry whose name was absolute (`/etc/x`, `C:\x`);
//! zip 9's `enclosed_name` instead drops a leading root or drive prefix and
//! returns the rest as a relative path. Such an entry is still contained in
//! the destination, but it is an archive that names a place outside it, so it
//! stays refused here exactly as before. Everything else (`..` leaving the
//! root, NUL, a separator hidden inside a component) is refused by zip itself.
use std::io::Read;
use std::path::PathBuf;
use zip::read::ZipFile;

/// True for a name that starts at a root or a drive instead of being relative.
fn names_absolute_place(name: &str) -> bool {
    if name.starts_with(['/', '\\']) {
        return true; // POSIX root, Windows root, UNC and `\\?\` prefixes
    }
    let bytes = name.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// The relative path below the destination, or `None` for an unsafe name.
pub(super) fn enclosed_name<R: Read + ?Sized>(entry: &ZipFile<'_, R>) -> Option<PathBuf> {
    let name = entry.name().ok()?;
    if names_absolute_place(&name) {
        return None;
    }
    entry.enclosed_name()
}
