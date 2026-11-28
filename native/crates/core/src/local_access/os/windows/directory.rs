//! Windows directory enumeration for the storage scanner.
//!
//! Every directory is read through `GetFileInformationByHandleEx` with the
//! extended (then full) information class, falling back to the ordinary
//! `read_dir` listing whenever a provider refuses a class or fails part-way.
//! Nothing here ends a directory early: a record that cannot be decoded is
//! skipped, an entry whose name cannot be a path is still counted, a reparse
//! tag that cannot be read leaves the entry an ordinary file or folder, and
//! names already handed out are never yielded twice after a fallback.
#[cfg(test)]
use super::super::EntryKind;
use super::super::LocalEntry;
use super::directory_records::{self, Layout};
use std::{
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
// is part of the Windows information-class contract. Every directory starts
// with 64 KiB (all a network share gets); a local directory that needs a
// second batch reads the rest in 1 MiB batches.
const BUFFER_WORDS: usize = 8192;
const MAX_BUFFER_WORDS: usize = 8192 * 16;

#[path = "directory_buffers.rs"]
mod buffers;

#[path = "directory_class.rs"]
mod class;
pub use class::{classify_open_file, metadata_class, metadata_is_link_like};
use class::{kind, link_like, reparse_tag_with_access};
#[cfg(test)]
use class::{reparse_tag, SPECIAL_TAGS};

#[cfg(test)]
#[path = "sync_link_task_tests.rs"]
mod sync_link_task_tests;

/// The information-class query behind every batch; tests substitute a
/// provider that fails part-way. Keep the owned iterator movable with its
/// pinned directory handle; captured query state must also be `Send`.
type Query<'a> =
    Box<dyn FnMut(&File, FILE_INFO_BY_HANDLE_CLASS, &mut [u64]) -> io::Result<()> + Send + 'a>;

pub struct Directory<'a> {
    path: PathBuf,
    file: File,
    buffer: Vec<u64>,
    cursor: Option<usize>,
    layout: Layout,
    started: bool,
    ended: bool,
    fallback: Option<std::fs::ReadDir>,
    /// Hashes of the names already yielded (8 bytes each); sorted and
    /// consulted only after a mid-way fallback so the ordinary listing does
    /// not repeat them.
    yielded: Vec<u64>,
    dedupe: bool,
    consented: bool,
    query: Query<'a>,
    /// A network share: 64 KiB batches, no per-file size corrections.
    remote: bool,
    /// Volume serial of this directory (asked once, when a file needs it).
    volume: Option<Option<u64>>,
}

impl Drop for Directory<'_> {
    fn drop(&mut self) {
        buffers::give_back(std::mem::take(&mut self.buffer));
    }
}

fn open(path: &Path, access: u32) -> io::Result<File> {
    let kind = if access & FILE_LIST_DIRECTORY != 0 {
        crate::local_access::protocol::ReadKind::Directory
    } else {
        crate::local_access::protocol::ReadKind::Metadata
    };
    super::read::open(path, kind)
}

pub(super) fn attributes(file: &File) -> io::Result<FILE_ATTRIBUTE_TAG_INFO> {
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

pub fn read_directory(path: &Path) -> io::Result<Directory<'static>> {
    read_directory_with_layout(path, Layout::Extended)
}

fn read_directory_with_layout(path: &Path, layout: Layout) -> io::Result<Directory<'static>> {
    read_directory_with_query(path, layout, query_directory)
}

fn read_directory_with_query<'a>(
    path: &Path,
    layout: Layout,
    query: impl FnMut(&File, FILE_INFO_BY_HANDLE_CLASS, &mut [u64]) -> io::Result<()> + Send + 'a,
) -> io::Result<Directory<'a>> {
    let file = open(path, FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)?;
    // The folder a volume is mounted in lists that volume's root (U16); the
    // folder's own (empty) directory is never what a location names.
    let (file, _mount_folder) = super::volume_mount::enter_mounted(
        file,
        crate::local_access::protocol::ReadKind::Directory,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
    )?;
    read_directory_from_file(path, file, layout, true, query)
}

/// Use a freshly opened handle under the caller's pinned ancestry. Ordinary
/// host scans never consult an unrelated GUI broker for reparse-tag probes.
pub(super) fn read_directory_handle(
    path: &Path,
    file: File,
    consented: bool,
) -> io::Result<Directory<'static>> {
    read_directory_from_file(path, file, Layout::Extended, consented, query_directory)
}

fn read_directory_from_file<'a>(
    path: &Path,
    file: File,
    layout: Layout,
    consented: bool,
    query: impl FnMut(&File, FILE_INFO_BY_HANDLE_CLASS, &mut [u64]) -> io::Result<()> + Send + 'a,
) -> io::Result<Directory<'a>> {
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
    // listing. The handle is the entry itself: a junction or symbolic link
    // named as the root lists its own (empty) directory, never its target;
    // a folder a volume is mounted in was replaced by that volume's root
    // before this (`volume_mount::enter_mounted`).
    let mut directory = Directory {
        path: path.to_path_buf(),
        file,
        buffer: buffers::take(BUFFER_WORDS),
        cursor: None,
        layout,
        started: false,
        ended: false,
        fallback: None,
        yielded: Vec::new(),
        dedupe: false,
        consented,
        query: Box::new(query),
        remote: buffers::is_remote(path),
        volume: None,
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
        if self.started && !self.remote && self.buffer.len() < MAX_BUFFER_WORDS {
            // A second batch: this local directory is large.
            buffers::give_back(std::mem::replace(
                &mut self.buffer,
                buffers::take(MAX_BUFFER_WORDS),
            ));
        }
        loop {
            // No clearing between batches: the decoder only follows offsets
            // the call wrote and checks every bound.
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
                buffers::give_back(std::mem::replace(&mut self.buffer, buffers::take(grown)));
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
                    self.start_fallback(listing);
                    return Ok(true);
                }
            }
            return Err(error);
        }
    }

    fn remember(&mut self, name: &OsString) {
        // Retained only while a mid-way fallback could still repeat names.
        if self.fallback.is_none() {
            self.yielded.push(buffers::name_hash(name));
        }
    }

    /// Continue with the ordinary listing, skipping what was yielded.
    fn start_fallback(&mut self, listing: std::fs::ReadDir) {
        self.yielded.sort_unstable();
        self.fallback = Some(listing);
        self.dedupe = true;
        self.cursor = None;
    }

    fn yielded_before(&self, name: &OsString) -> bool {
        self.yielded
            .binary_search(&buffers::name_hash(name))
            .is_ok()
    }

    /// The volume serial of this directory, asked at most once.
    fn volume(&mut self) -> Option<u64> {
        *self
            .volume
            .get_or_insert_with(|| buffers::volume_serial(&self.file))
    }

    fn next_entry(&mut self) -> io::Result<Option<LocalEntry>> {
        loop {
            if let Some(fallback) = &mut self.fallback {
                let Some(entry) = fallback.next() else {
                    return Ok(None);
                };
                let entry = entry?;
                let name = entry.file_name();
                if self.dedupe && self.yielded_before(&name) {
                    continue;
                }
                let metadata = entry.metadata().map_err(|error| {
                    super::super::EntryError::wrap(name.clone(), &entry.path(), error)
                })?;
                use std::os::windows::fs::MetadataExt;
                let attrs = metadata.file_attributes();
                let tag = if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    reparse_tag_with_access(&entry.path(), self.consented)
                } else {
                    0
                };
                return Ok(Some(LocalEntry {
                    name,
                    kind: kind(attrs, tag),
                    is_dir: attrs & FILE_ATTRIBUTE_DIRECTORY != 0,
                    is_link_like: link_like(attrs, tag),
                    size: metadata.len(),
                    // The ordinary listing exposes no allocation size.
                    allocated: None,
                    hardlink: None,
                    device: None,
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
                            self.start_fallback(listing);
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
                    None => reparse_tag_with_access(&self.path.join(&record.name), self.consented),
                }
            } else {
                0
            };
            self.remember(&record.name);
            let file = record.attributes & FILE_ATTRIBUTE_DIRECTORY == 0;
            let allocated = if file && buffers::needs_compressed_size(&record, tag, self.remote) {
                buffers::compressed_size(&self.path.join(&record.name)).unwrap_or(record.allocated)
            } else {
                record.allocated
            };
            // The records carry no link count: every file that occupies
            // space carries its identity, so a walk counts each MFT record
            // once (hard links share it).
            let hardlink = match record.file_id {
                Some(id) if file && allocated > 0 => self
                    .volume()
                    .map(|volume| (volume, (id as u64) ^ ((id >> 64) as u64))),
                _ => None,
            };
            return Ok(Some(LocalEntry {
                name: record.name,
                size: record.size,
                allocated: Some(allocated),
                hardlink,
                device: None,
                kind: kind(record.attributes, tag),
                is_dir: record.attributes & FILE_ATTRIBUTE_DIRECTORY != 0,
                is_link_like: link_like(record.attributes, tag),
                unreachable: record.unrepresentable,
                mtime_ms: record.mtime_ms,
                btime_ms: record.btime_ms,
                hidden: record.attributes & FILE_ATTRIBUTE_HIDDEN != 0,
                system: record.attributes & FILE_ATTRIBUTE_SYSTEM != 0,
            }));
        }
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
