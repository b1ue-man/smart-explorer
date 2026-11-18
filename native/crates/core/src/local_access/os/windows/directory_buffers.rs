//! Per-thread batch buffers of the directory reader, the drive kind of a
//! listing, name hashes for the fallback, the volume serial for hard-link
//! identities and the occupied size of compressed or sparse files.
use super::directory_records::Record;
use std::{
    cell::RefCell,
    ffi::OsString,
    fs::File,
    hash::{Hash, Hasher},
    os::windows::{ffi::OsStrExt, io::AsRawHandle},
    path::Path,
};
use windows_sys::Win32::Storage::FileSystem::{
    FileIdInfo, GetCompressedFileSizeW, GetDriveTypeW, GetFileInformationByHandleEx,
    FILE_ATTRIBUTE_COMPRESSED, FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_SPARSE_FILE, FILE_ID_INFO,
};

/// Buffers a thread keeps for its next directories.
const KEPT: usize = 4;
/// `DRIVE_REMOTE` of `GetDriveTypeW`.
const DRIVE_REMOTE: u32 = 4;
/// `FILE_ATTRIBUTE_RECALL_ON_OPEN` / `_ON_DATA_ACCESS`: cloud placeholders,
/// whose zero allocation is the truth.
const RECALL: u32 = 0x0004_0000 | 0x0040_0000;
/// `IO_REPARSE_TAG_WOF` (CompactOS) and `IO_REPARSE_TAG_DEDUP`: the data
/// lives in a side stream or the chunk store, not in the record's figure.
const SIDE_STORED_TAGS: [u32; 2] = [0x8000_0017, 0x8000_0013];
/// Larger files cannot be resident in their MFT record.
const RESIDENT_MAX: u64 = 4096;

thread_local! {
    static POOL: RefCell<Vec<Vec<u64>>> = const { RefCell::new(Vec::new()) };
    /// `GetDriveTypeW` per drive letter, asked once per thread.
    static DRIVES: RefCell<[Option<bool>; 26]> = const { RefCell::new([None; 26]) };
}

/// A buffer of `words` from this thread's pool (zeroed when new).
pub(super) fn take(words: usize) -> Vec<u64> {
    POOL.with(|pool| {
        let mut pool = pool.borrow_mut();
        match pool.iter().position(|buffer| buffer.len() == words) {
            Some(index) => pool.swap_remove(index),
            None => vec![0; words],
        }
    })
}

pub(super) fn give_back(buffer: Vec<u64>) {
    if buffer.is_empty() {
        return;
    }
    POOL.with(|pool| {
        let mut pool = pool.borrow_mut();
        if pool.len() < KEPT {
            pool.push(buffer);
        }
    });
}

/// UNC paths and mapped network drives.
pub(super) fn is_remote(path: &Path) -> bool {
    let text = path.to_string_lossy();
    let rest = text.strip_prefix(r"\\?\").unwrap_or(&text);
    if rest.starts_with(r"UNC\") || (!text.starts_with(r"\\?\") && text.starts_with(r"\\")) {
        return true;
    }
    let mut chars = rest.chars();
    let (Some(letter), Some(':')) = (chars.next(), chars.next()) else {
        return false;
    };
    if !letter.is_ascii_alphabetic() {
        return false;
    }
    let index = (letter.to_ascii_uppercase() as u8 - b'A') as usize;
    DRIVES.with(|drives| {
        *drives.borrow_mut()[index].get_or_insert_with(|| {
            let root: Vec<u16> = format!("{letter}:\\")
                .encode_utf16()
                .chain(Some(0))
                .collect();
            unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_REMOTE }
        })
    })
}

/// A stable 64-bit hash of a name (fixed SipHash keys).
pub(super) fn name_hash(name: &OsString) -> u64 {
    // `new()` uses fixed keys: equal names hash equally in this process.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    hasher.finish()
}

/// The serial of the volume holding `directory`.
pub(super) fn volume_serial(directory: &File) -> Option<u64> {
    // SAFETY: plain data, filled by the call below.
    let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        GetFileInformationByHandleEx(
            directory.as_raw_handle(),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    (ok != 0).then_some(info.VolumeSerialNumber)
}

/// A local file whose record says it occupies nothing although it cannot be
/// resident: compressed, sparse or stored beside its record (WOF, dedup).
/// Cloud placeholders and offline files really occupy nothing.
pub(super) fn needs_compressed_size(record: &Record, tag: u32, remote: bool) -> bool {
    let side_stored = SIDE_STORED_TAGS.contains(&tag);
    !remote
        && !record.unrepresentable
        && record.allocated == 0
        && record.size > RESIDENT_MAX
        && record.attributes & (RECALL | FILE_ATTRIBUTE_OFFLINE) == 0
        && (side_stored
            || record.attributes & (FILE_ATTRIBUTE_COMPRESSED | FILE_ATTRIBUTE_SPARSE_FILE) != 0)
}

/// `GetCompressedFileSizeW`: the bytes the file really occupies.
pub(super) fn compressed_size(path: &Path) -> Option<u64> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut high = 0u32;
    let low = unsafe { GetCompressedFileSizeW(wide.as_ptr(), &mut high) };
    if low == u32::MAX && std::io::Error::last_os_error().raw_os_error() != Some(0) {
        return None;
    }
    Some((u64::from(high) << 32) | u64::from(low))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyse_task_windows_remote_paths_and_name_hashes() {
        assert!(is_remote(Path::new(r"\\server\share\dir")));
        assert!(is_remote(Path::new(r"\\?\UNC\server\share")));
        assert!(!is_remote(Path::new(r"\\?\C:\Windows")));
        let a = OsString::from("Datei.txt");
        assert_eq!(name_hash(&a), name_hash(&OsString::from("Datei.txt")));
        assert_ne!(name_hash(&a), name_hash(&OsString::from("datei.txt")));
        let buffer = take(16);
        assert_eq!(buffer.len(), 16);
        give_back(buffer);
        assert_eq!(take(16).len(), 16);
    }
}
