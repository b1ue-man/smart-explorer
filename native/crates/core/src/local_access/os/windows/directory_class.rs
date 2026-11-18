//! Classification of directory entries: link boundaries, special files and
//! the reparse tag of an entry the records did not name.
use super::super::super::EntryKind;
use super::{attributes, open};
use std::{fs::File, io, path::Path};
use windows_sys::Win32::Storage::FileSystem::*;

const NAME_SURROGATE: u32 = 0x2000_0000;
// Special-file reparse tags (no name-surrogate bit, no data stream to read):
// AF_UNIX sockets and the FIFO/character/block nodes WSL stores on NTFS.
// Values as in windows-sys 0.59 `IO_REPARSE_TAG_AF_UNIX` / `IO_REPARSE_TAG_LX_*`.
const SPECIAL_TAGS: [u32; 4] = [0x8000_0023, 0x8000_0024, 0x8000_0025, 0x8000_0026];

/// The reparse tag of `path`, or 0 when it cannot be read: an unreadable tag
/// classifies the entry as an ordinary file or folder instead of dropping it.
pub(super) fn reparse_tag(path: &Path) -> u32 {
    reparse_tag_with_access(path, true)
}

pub(super) fn reparse_tag_with_access(path: &Path, consented: bool) -> u32 {
    let file = if consented {
        open(path, FILE_READ_ATTRIBUTES)
    } else {
        super::super::read::open_direct(
            path,
            crate::local_access::protocol::ReadKind::Metadata,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        )
    };
    file.and_then(|file| attributes(&file))
        .map(|info| info.ReparseTag)
        .unwrap_or(0)
}

pub(super) fn link_like(attributes: u32, tag: u32) -> bool {
    attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 && (tag == 0 || tag & NAME_SURROGATE != 0)
}

/// Data reparse points (for example cloud placeholders) are ordinary entries.
/// A missing tag remains a boundary for VFS operations, even when the Explorer
/// can still display the entry using its ordinary file/directory attributes.
pub fn metadata_is_link_like(path: &Path, metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    let attrs = metadata.file_attributes();
    metadata.is_symlink()
        || (attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 && link_like(attrs, reparse_tag(path)))
}

/// Link boundary and special class with at most one reparse-tag read.
pub fn metadata_class(
    path: &Path,
    metadata: &std::fs::Metadata,
) -> crate::local_access::MetadataClass {
    use std::os::windows::fs::MetadataExt;
    let attrs = metadata.file_attributes();
    let tag = if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        reparse_tag(path)
    } else {
        0
    };
    let boundary = metadata.is_symlink()
        || (attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 && link_like(attrs, tag));
    crate::local_access::MetadataClass {
        link_like: boundary,
        special: !boundary && kind(attrs, tag) == EntryKind::Other,
    }
}

/// The classification of this handle, even if its former path was replaced.
pub fn classify_open_file(file: &File) -> io::Result<crate::local_access::MetadataClass> {
    use std::os::windows::fs::MetadataExt;
    let metadata = file.metadata()?;
    let attrs = metadata.file_attributes();
    let tag = if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        attributes(file)?.ReparseTag
    } else {
        0
    };
    let boundary = metadata.is_symlink() || link_like(attrs, tag);
    Ok(crate::local_access::MetadataClass {
        link_like: boundary,
        special: !boundary && kind(attrs, tag) == EntryKind::Other,
    })
}

pub(super) fn kind(attributes: u32, tag: u32) -> EntryKind {
    let reparse = attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    if reparse && tag & NAME_SURROGATE != 0 {
        EntryKind::Link
    } else if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        EntryKind::Directory
    } else if attributes & FILE_ATTRIBUTE_DEVICE != 0 || (reparse && SPECIAL_TAGS.contains(&tag)) {
        EntryKind::Other
    } else {
        EntryKind::File
    }
}
