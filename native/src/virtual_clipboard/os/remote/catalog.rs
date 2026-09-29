//! The file list Explorer receives: which listed entries it can take, why
//! the others are left out, and the FILEGROUPDESCRIPTORW describing them.
use crate::transfer::{ListedEntry, SelectionListing};
use windows::core::{Error, Result};
use windows::Win32::Foundation::{GlobalFree, FILETIME, HGLOBAL, STG_E_MEDIUMFULL};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE, GMEM_ZEROINIT,
};
use windows::Win32::UI::Shell::{
    FD_ATTRIBUTES, FD_FILESIZE, FD_PROGRESSUI, FD_WRITESTIME, FILEDESCRIPTORW,
};

/// `cFileName` holds 260 UTF-16 units including its NUL, so a relative path
/// may use at most 259 of them; longer ones cannot reach Explorer this way.
const NAME_UNITS: usize = 260;
/// FILEGROUPDESCRIPTORW: a u32 count followed by packed 592-byte entries.
const COUNT_BYTES: usize = std::mem::size_of::<u32>();
const ENTRY_BYTES: usize = std::mem::size_of::<FILEDESCRIPTORW>();

pub(super) const TOO_LARGE_NOTE: &str =
    "Auswahl zu groß für den Explorer – bitte in Smart Explorer einfügen";
const INVALID_NAME: &str = "Der Name enthält Zeichen, die Windows nicht zulässt";
const RESERVED_NAME: &str =
    "Windows reserviert diesen Namen – in Smart Explorer einfügen überträgt ihn";
const TOO_LONG_REASON: &str =
    "Pfad ab 260 Zeichen – nicht an den Explorer übergeben; in Smart Explorer einfügen überträgt ihn";
/// Omitted entries listed with their reasons, as many as a transfer lists.
const LISTED_ISSUES: usize = 100;

/// Allocates the global block a descriptor is written into.
pub(super) type DescriptorAlloc = fn(usize) -> Result<HGLOBAL>;

pub(super) fn allocate_global(bytes: usize) -> Result<HGLOBAL> {
    unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes) }
}

/// The entries Explorer gets, in descriptor order (folders before their
/// contents, as listed), and what had to be left out.
pub(super) struct Catalog {
    pub(super) entries: Vec<ListedEntry>,
    pub(super) files: u64,
    /// Relative paths of 260 or more UTF-16 units.
    pub(super) too_long: Vec<String>,
    /// Entries that could not be listed or represented, with the reason.
    pub(super) problems: Vec<(String, String)>,
    /// False when the listing stopped early (the hand-off ended).
    pub(super) complete: bool,
}

impl Catalog {
    pub(super) fn from_listing(listing: SelectionListing) -> Self {
        let SelectionListing {
            entries: listed,
            problems,
            complete,
            ..
        } = listing;
        let mut catalog = Self {
            entries: Vec::with_capacity(listed.len()),
            files: 0,
            too_long: Vec::new(),
            problems,
            complete,
        };
        for entry in listed {
            if let Some(reason) = name_problem(&entry.rel) {
                catalog.problems.push((entry.path, reason.to_string()));
            } else if entry.rel.encode_utf16().count() >= NAME_UNITS {
                catalog.too_long.push(entry.rel);
            } else {
                catalog.files += u64::from(!entry.is_dir);
                catalog.entries.push(entry);
            }
        }
        catalog
    }

    /// Entries left out count as errors of the hand-off.
    pub(super) fn errors(&self) -> u64 {
        (self.too_long.len() + self.problems.len()) as u64
    }

    /// The omitted entries with their reasons for the transfer list; the
    /// first ones only, as a transfer lists them (`errors` counts all).
    pub(super) fn issues(&self) -> Vec<(String, String)> {
        self.too_long
            .iter()
            .map(|rel| (rel.clone(), TOO_LONG_REASON.to_string()))
            .chain(self.problems.iter().cloned())
            .take(LISTED_ISSUES)
            .collect()
    }

    /// Why entries are missing, if any are.
    pub(super) fn note(&self) -> Option<String> {
        if !self.too_long.is_empty() {
            return Some(format!(
                "{} mit Pfaden ab 260 Zeichen nicht an den Explorer übergeben – in Smart Explorer einfügen überträgt sie",
                entries_text(self.too_long.len())
            ));
        }
        let (path, message) = self.problems.first()?;
        Some(match self.problems.len() - 1 {
            0 => format!("{path}: {message}"),
            more => format!("{path}: {message} (und {more} weitere)"),
        })
    }
}

/// Why Explorer could not create `rel` as named, if so. Explorer turns the
/// descriptor name into a path as it is: a server's `..` would leave the
/// target folder, `name:stream` would write an NTFS stream, a backslash would
/// split the name, and Win32 strips trailing dots and spaces or opens a device.
fn name_problem(rel: &str) -> Option<&'static str> {
    for part in rel.split('/') {
        let invalid = |c: char| c < ' ' || "<>:\"|?*\\".contains(c);
        if part.is_empty() || part == "." || part == ".." || part.contains(invalid) {
            return Some(INVALID_NAME);
        }
        if part.ends_with('.') || part.ends_with(' ') || is_device_name(part) {
            return Some(RESERVED_NAME);
        }
    }
    None
}

/// Reserved device names (Microsoft's file naming rules), which Win32
/// resolves in any folder and with any extension.
fn is_device_name(part: &str) -> bool {
    let stem = part
        .split('.')
        .next()
        .unwrap_or(part)
        .trim_end()
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let mut digit = stem.get(3..).unwrap_or_default().chars();
    matches!(stem.get(..3), Some("COM" | "LPT"))
        && matches!(digit.next(), Some('0'..='9' | '¹' | '²' | '³'))
        && digit.next().is_none()
}

fn entries_text(count: usize) -> String {
    if count == 1 {
        "1 Eintrag".to_string()
    } else {
        format!("{count} Einträge")
    }
}

/// Writes the descriptor of `entries` into a new global block the caller
/// owns. A block Windows cannot provide is `STG_E_MEDIUMFULL`, the documented
/// GetData answer when the medium cannot hold the data.
pub(super) fn render(entries: &[ListedEntry], alloc: DescriptorAlloc) -> Result<HGLOBAL> {
    let medium_full = || Error::new(STG_E_MEDIUMFULL, TOO_LARGE_NOTE);
    let count = u32::try_from(entries.len()).map_err(|_| medium_full())?;
    let bytes = entries
        .len()
        .max(1)
        .checked_mul(ENTRY_BYTES)
        .and_then(|bytes| bytes.checked_add(COUNT_BYTES))
        .ok_or_else(medium_full)?;
    let handle = alloc(bytes).map_err(|_| medium_full())?;
    let base = unsafe { GlobalLock(handle) }.cast::<u8>();
    if base.is_null() {
        let error = Error::from_win32();
        // Still owned here: free it once; 0.58 projects the NULL success as Err.
        let _ = unsafe { GlobalFree(handle) };
        return Err(error);
    }
    // SAFETY: the locked block holds 4 + max(n, 1) × 592 bytes; every write
    // stays inside it, unaligned because the Shell layout is packed.
    unsafe {
        std::ptr::write_unaligned(base.cast::<u32>(), count);
        for (index, entry) in entries.iter().enumerate() {
            let at = base.add(COUNT_BYTES + index * ENTRY_BYTES);
            std::ptr::write_unaligned(at.cast::<FILEDESCRIPTORW>(), describe(entry));
        }
        let _ = GlobalUnlock(handle);
    }
    Ok(handle)
}

fn describe(entry: &ListedEntry) -> FILEDESCRIPTORW {
    // Explorer recreates the folders named in the relative path.
    let mut name = [0u16; NAME_UNITS];
    for (slot, unit) in name.iter_mut().zip(entry.rel.encode_utf16()) {
        *slot = if unit == u16::from(b'/') {
            u16::from(b'\\')
        } else {
            unit
        };
    }
    let mut flags = FD_PROGRESSUI.0 as u32;
    let mut attributes = 0;
    let (mut high, mut low) = (0, 0);
    if entry.is_dir {
        // Creates the folder even when it stays empty.
        flags |= FD_ATTRIBUTES.0 as u32;
        attributes = FILE_ATTRIBUTE_DIRECTORY.0;
    } else if entry.size_known {
        // Also required, as 0/0, for Explorer to create an empty file.
        flags |= FD_FILESIZE.0 as u32;
        (high, low) = ((entry.size >> 32) as u32, entry.size as u32);
    }
    let mut written = FILETIME::default();
    if entry.mtime_ms != 0 {
        flags |= FD_WRITESTIME.0 as u32;
        written = crate::virtual_clipboard::imp::filetime_from_ms(entry.mtime_ms);
    }
    FILEDESCRIPTORW {
        dwFlags: flags,
        dwFileAttributes: attributes,
        ftLastWriteTime: written,
        nFileSizeHigh: high,
        nFileSizeLow: low,
        cFileName: name,
        ..Default::default()
    }
}
