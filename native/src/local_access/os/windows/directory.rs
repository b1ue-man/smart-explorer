//! Windows directory enumeration for the storage scanner.
//!
//! Every directory is read through `GetFileInformationByHandleEx` with the
//! extended (then full) information class, falling back to the ordinary
//! `read_dir` listing whenever a provider refuses a class or fails part-way.
//! Nothing here ends a directory early: a record that cannot be decoded is
//! skipped, an entry whose name cannot be a path is still counted, a reparse
//! tag that cannot be read leaves the entry an ordinary file or folder, and
//! names already handed out are never yielded twice after a fallback.
use super::super::{EntryKind, LocalEntry};
use super::directory_records::{self, Layout};
use std::{
    collections::HashSet,
    ffi::OsString,
    fs::File,
    io,
    mem::size_of,
    os::windows::io::AsRawHandle,
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Foundation::{ERROR_HANDLE_EOF, ERROR_MORE_DATA, ERROR_NO_MORE_FILES},
    Storage::FileSystem::*,
};

// A directory batch, not a limit on entries or traversal. Eight-byte alignment
// is part of the Windows information-class contract.
const BUFFER_WORDS: usize = 8192;
const MAX_BUFFER_WORDS: usize = 8192 * 16;
const NAME_SURROGATE: u32 = 0x2000_0000;

/// The information-class query behind every batch; tests substitute a
/// provider that fails part-way.
type Query<'a> =
    Box<dyn FnMut(&File, FILE_INFO_BY_HANDLE_CLASS, &mut [u64]) -> io::Result<()> + 'a>;

pub(crate) struct Directory<'a> {
    path: PathBuf,
    file: File,
    buffer: Vec<u64>,
    cursor: Option<usize>,
    layout: Layout,
    started: bool,
    ended: bool,
    fallback: Option<std::fs::ReadDir>,
    /// Names already yielded; only consulted after a mid-way fallback so the
    /// ordinary listing does not repeat them.
    yielded: HashSet<OsString>,
    dedupe: bool,
    query: Query<'a>,
}

fn open(path: &Path, access: u32) -> io::Result<File> {
    let kind = if access & FILE_LIST_DIRECTORY != 0 {
        crate::local_access::protocol::ReadKind::Directory
    } else {
        crate::local_access::protocol::ReadKind::Metadata
    };
    super::read::open(path, kind)
}

fn attributes(file: &File) -> io::Result<FILE_ATTRIBUTE_TAG_INFO> {
    let mut result = FILE_ATTRIBUTE_TAG_INFO {
        FileAttributes: 0,
        ReparseTag: 0,
    };
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            (&mut result as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(result)
}

fn unsupported(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(1 | 50 | 87 | 124))
}

fn enumeration_ended(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(code) if code == ERROR_NO_MORE_FILES as i32 || code == ERROR_HANDLE_EOF as i32
    )
}

pub(crate) fn read_directory(path: &Path) -> io::Result<Directory<'static>> {
    read_directory_with_layout(path, Layout::Extended)
}

fn read_directory_with_layout(path: &Path, layout: Layout) -> io::Result<Directory<'static>> {
    read_directory_with_query(path, layout, query_directory)
}

fn read_directory_with_query<'a>(
    path: &Path,
    layout: Layout,
    query: impl FnMut(&File, FILE_INFO_BY_HANDLE_CLASS, &mut [u64]) -> io::Result<()> + 'a,
) -> io::Result<Directory<'a>> {
    let file = open(path, FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)?;
    // The attribute probe is advisory: a provider that cannot answer it still
    // gets to enumerate, and a non-directory shows up as an enumeration error.
    if let Ok(info) = attributes(&file) {
        if info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "Scan-Ziel ist kein Ordner",
            ));
        }
    }
    // Link-like children are classified (and skipped) by the parent's
    // listing; the directory the caller asked for is always entered, so a
    // junction chosen as the scan root still works.
    let mut directory = Directory {
        path: path.to_path_buf(),
        file,
        buffer: vec![0; BUFFER_WORDS],
        cursor: None,
        layout,
        started: false,
        ended: false,
        fallback: None,
        yielded: HashSet::new(),
        dedupe: false,
        query: Box::new(query),
    };
    // Establish the listing here so callers can distinguish a failed root
    // from an individual entry that fails after enumeration has begun.
    if !directory.refill()? {
        directory.ended = true;
    }
    Ok(directory)
}

fn query_directory(
    file: &File,
    class: FILE_INFO_BY_HANDLE_CLASS,
    buffer: &mut [u64],
) -> io::Result<()> {
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            class,
            buffer.as_mut_ptr().cast(),
            std::mem::size_of_val(buffer) as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

impl Directory<'_> {
    fn refill(&mut self) -> io::Result<bool> {
        loop {
            self.buffer.fill(0);
            let class = match (self.layout, self.started) {
                (Layout::Extended, false) => FileIdExtdDirectoryRestartInfo,
                (Layout::Extended, true) => FileIdExtdDirectoryInfo,
                (Layout::Full, false) => FileFullDirectoryRestartInfo,
                (Layout::Full, true) => FileFullDirectoryInfo,
            };
            let error = match (self.query)(&self.file, class, &mut self.buffer) {
                Ok(()) => {
                    self.started = true;
                    self.cursor = Some(0);
                    return Ok(true);
                }
                Err(error) => error,
            };
            if enumeration_ended(&error) {
                return Ok(false);
            }
            if error.raw_os_error() == Some(ERROR_MORE_DATA as i32)
                && self.buffer.len() < MAX_BUFFER_WORDS
            {
                // One record did not fit: grow the batch and ask again.
                let grown = (self.buffer.len() * 4).min(MAX_BUFFER_WORDS);
                self.buffer = vec![0; grown];
                continue;
            }
            if !self.started && unsupported(&error) {
                if matches!(self.layout, Layout::Extended) {
                    self.layout = Layout::Full;
                    continue;
                }
                // Providers without either information class retain the ordinary
                // listing route. A denied fallback remains a visible scan error.
                self.fallback = Some(std::fs::read_dir(&self.path)?);
                return Ok(true);
            }
            if self.started {
                // A provider that stops answering part-way must not cost the
                // rest of the directory: continue through the ordinary
                // listing, skipping what was already yielded.
                if let Ok(listing) = std::fs::read_dir(&self.path) {
                    self.fallback = Some(listing);
                    self.dedupe = true;
                    self.cursor = None;
                    return Ok(true);
                }
            }
            return Err(error);
        }
    }

    fn remember(&mut self, name: &OsString) {
        // Retained only while a mid-way fallback could still repeat names.
        if self.fallback.is_none() {
            self.yielded.insert(name.clone());
        }
    }

    fn next_entry(&mut self) -> io::Result<Option<LocalEntry>> {
        loop {
            if let Some(fallback) = &mut self.fallback {
                let Some(entry) = fallback.next() else {
                    return Ok(None);
                };
                let entry = entry?;
                let name = entry.file_name();
                if self.dedupe && self.yielded.contains(&name) {
                    continue;
                }
                let metadata = entry.metadata().map_err(|error| {
                    io::Error::new(error.kind(), format!("{}: {error}", entry.path().display()))
                })?;
                use std::os::windows::fs::MetadataExt;
                let attrs = metadata.file_attributes();
                let tag = if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    reparse_tag(&entry.path())
                } else {
                    0
                };
                return Ok(Some(LocalEntry {
                    name,
                    kind: kind(attrs, tag),
                    is_dir: attrs & FILE_ATTRIBUTE_DIRECTORY != 0,
                    is_link_like: attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0,
                    size: metadata.len(),
                    unreachable: false,
                    mtime_ms: metadata
                        .modified()
                        .map(crate::local_access::system_time_ms)
                        .unwrap_or(0),
                    btime_ms: metadata
                        .created()
                        .map(crate::local_access::system_time_ms)
                        .unwrap_or(0),
                    hidden: attrs & FILE_ATTRIBUTE_HIDDEN != 0,
                    system: attrs & FILE_ATTRIBUTE_SYSTEM != 0,
                }));
            }
            if self.cursor.is_none() {
                match self.refill() {
                    Ok(false) => return Ok(None),
                    Ok(true) => {}
                    Err(error) => {
                        self.ended = true;
                        return Err(error);
                    }
                }
            }
            let Some(offset) = self.cursor else {
                // The query selected the ordinary enumeration fallback.
                continue;
            };
            // The API owns the initialized, aligned allocation only during the
            // synchronous call. Decode bounds-checked bytes, never cast records.
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    self.buffer.as_ptr().cast::<u8>(),
                    self.buffer.len() * size_of::<u64>(),
                )
            };
            let record = match directory_records::decode(bytes, offset, self.layout) {
                Ok(record) => record,
                Err(directory_records::DecodeError {
                    next: Some(next),
                    error,
                }) => {
                    // One malformed record is skipped, not the directory.
                    self.cursor = Some(next);
                    return Err(io::Error::new(
                        error.kind(),
                        format!("{}: {error}", self.path.display()),
                    ));
                }
                Err(directory_records::DecodeError { next: None, error }) => {
                    // Without a usable header the batch is lost; the ordinary
                    // listing finishes the directory without repeats.
                    match std::fs::read_dir(&self.path) {
                        Ok(listing) => {
                            self.fallback = Some(listing);
                            self.dedupe = true;
                            self.cursor = None;
                            return Err(error);
                        }
                        Err(_) => {
                            self.ended = true;
                            return Err(error);
                        }
                    }
                }
            };
            self.cursor = record.next;
            if record.name == "." || record.name == ".." {
                continue;
            }
            let tag = if record.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                match record.tag {
                    Some(tag) => tag,
                    None if record.unrepresentable => 0,
                    None => reparse_tag(&self.path.join(&record.name)),
                }
            } else {
                0
            };
            self.remember(&record.name);
            return Ok(Some(LocalEntry {
                name: record.name,
                size: record.size,
                kind: kind(record.attributes, tag),
                is_dir: record.attributes & FILE_ATTRIBUTE_DIRECTORY != 0,
                is_link_like: record.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0,
                unreachable: record.unrepresentable,
                mtime_ms: record.mtime_ms,
                btime_ms: record.btime_ms,
                hidden: record.attributes & FILE_ATTRIBUTE_HIDDEN != 0,
                system: record.attributes & FILE_ATTRIBUTE_SYSTEM != 0,
            }));
        }
    }
}

/// The reparse tag of `path`, or 0 when it cannot be read: an unreadable tag
/// classifies the entry as an ordinary file or folder instead of dropping it.
fn reparse_tag(path: &Path) -> u32 {
    open(path, FILE_READ_ATTRIBUTES)
        .and_then(|file| attributes(&file))
        .map(|info| info.ReparseTag)
        .unwrap_or(0)
}

fn kind(attributes: u32, tag: u32) -> EntryKind {
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 && tag & NAME_SURROGATE != 0 {
        EntryKind::Link
    } else if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        EntryKind::Directory
    } else if attributes & FILE_ATTRIBUTE_DEVICE != 0 {
        EntryKind::Other
    } else {
        EntryKind::File
    }
}

impl Iterator for Directory<'_> {
    type Item = io::Result<LocalEntry>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.ended {
            return None;
        }
        match self.next_entry() {
            Ok(Some(entry)) => Some(Ok(entry)),
            Ok(None) => {
                self.ended = true;
                None
            }
            Err(error) => Some(Err(error)),
        }
    }
}

#[cfg(test)]
#[path = "directory_tests.rs"]
mod tests;
