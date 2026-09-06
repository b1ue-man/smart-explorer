use super::super::{EntryKind, LocalEntry};
use super::{
    directory_records::{self, Layout},
    privilege::BackupRead,
};
use std::{
    fs::{File, OpenOptions},
    io,
    mem::size_of,
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
};
use windows_sys::Win32::{Foundation::ERROR_NO_MORE_FILES, Storage::FileSystem::*};

// A directory batch, not a limit on entries or traversal. Eight-byte alignment
// is part of the Windows information-class contract.
const BUFFER_WORDS: usize = 8192;
const NAME_SURROGATE: u32 = 0x2000_0000;

pub(in crate::analytics::os) struct Directory {
    path: PathBuf,
    file: File,
    buffer: Vec<u64>,
    cursor: Option<usize>,
    layout: Layout,
    started: bool,
    ended: bool,
    fallback: Option<std::fs::ReadDir>,
}

fn open(path: &Path, access: u32) -> io::Result<File> {
    let attempt = || {
        OpenOptions::new()
            .access_mode(access)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
    };
    match attempt() {
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            // Keep the original path/access error if this token lacks backup rights.
            let Ok(_backup) = BackupRead::enable() else {
                return Err(error);
            };
            attempt()
        }
        result => result,
    }
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

pub(in crate::analytics::os) fn read_directory(path: &Path) -> io::Result<Directory> {
    let file = open(path, FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES)?;
    let info = attributes(&file)?;
    if info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "Scan-Ziel ist kein Ordner",
        ));
    }
    // Check the opened object too: a child may have become a junction since
    // the parent was enumerated. Never resolve that last reparse component.
    if info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        && info.ReparseTag & NAME_SURROGATE != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Verzeichnisverknüpfung wird nicht verfolgt",
        ));
    }
    Ok(Directory {
        path: path.to_path_buf(),
        file,
        buffer: vec![0; BUFFER_WORDS],
        cursor: None,
        layout: Layout::Extended,
        started: false,
        ended: false,
        fallback: None,
    })
}

impl Directory {
    fn refill(&mut self) -> io::Result<bool> {
        loop {
            self.buffer.fill(0);
            let class = match (self.layout, self.started) {
                (Layout::Extended, false) => FileIdExtdDirectoryRestartInfo,
                (Layout::Extended, true) => FileIdExtdDirectoryInfo,
                (Layout::Full, false) => FileFullDirectoryRestartInfo,
                (Layout::Full, true) => FileFullDirectoryInfo,
            };
            if unsafe {
                GetFileInformationByHandleEx(
                    self.file.as_raw_handle(),
                    class,
                    self.buffer.as_mut_ptr().cast(),
                    (self.buffer.len() * size_of::<u64>()) as u32,
                )
            } != 0
            {
                self.started = true;
                self.cursor = Some(0);
                return Ok(true);
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                return Ok(false);
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
            return Err(error);
        }
    }

    fn next_entry(&mut self) -> io::Result<Option<LocalEntry>> {
        loop {
            if let Some(fallback) = &mut self.fallback {
                let Some(entry) = fallback.next() else {
                    return Ok(None);
                };
                let entry = entry?;
                let metadata = entry.metadata().map_err(|error| {
                    io::Error::new(error.kind(), format!("{}: {error}", entry.path().display()))
                })?;
                use std::os::windows::fs::MetadataExt;
                let attrs = metadata.file_attributes();
                let tag = if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    let path = entry.path();
                    Some(
                        open(&path, FILE_READ_ATTRIBUTES)
                            .and_then(|file| attributes(&file))
                            .map_err(|error| {
                                io::Error::new(error.kind(), format!("{}: {error}", path.display()))
                            })?
                            .ReparseTag,
                    )
                } else {
                    None
                };
                return Ok(Some(LocalEntry {
                    name: entry.file_name(),
                    kind: kind(attrs, tag.unwrap_or(0)),
                    size: metadata.len(),
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
                // The first query selected the ordinary enumeration fallback.
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
                Err(error) => {
                    self.ended = true;
                    return Err(error);
                }
            };
            self.cursor = record.next;
            if record.name == "." || record.name == ".." {
                continue;
            }
            let tag = if record.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                match record.tag {
                    Some(tag) => tag,
                    None => {
                        let path = self.path.join(&record.name);
                        open(&path, FILE_READ_ATTRIBUTES)
                            .and_then(|file| attributes(&file))
                            .map_err(|error| {
                                io::Error::new(error.kind(), format!("{}: {error}", path.display()))
                            })?
                            .ReparseTag
                    }
                }
            } else {
                0
            };
            return Ok(Some(LocalEntry {
                name: record.name,
                size: record.size,
                kind: kind(record.attributes, tag),
            }));
        }
    }
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

impl Iterator for Directory {
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
mod tests {
    use super::*;
    #[test]
    fn analytics_access_task_full_record_fallback_and_reparse_classification() {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::write(fixture.path().join("file"), vec![0; 11]).unwrap();
        std::fs::create_dir(fixture.path().join("sub")).unwrap();
        let mut full = read_directory(fixture.path()).unwrap();
        full.layout = Layout::Full;
        let entries: Vec<_> = full.collect::<io::Result<_>>().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.name == "file")
                .unwrap()
                .size,
            11
        );
        assert_eq!(
            kind(
                FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT,
                0xa0000003
            ),
            EntryKind::Link
        );
        assert_eq!(
            kind(
                FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT,
                0xa000000c
            ),
            EntryKind::Link
        );
        assert_eq!(
            kind(
                FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT,
                0x9000001a
            ),
            EntryKind::Directory
        );
        for code in [1, 50, 87, 124] {
            assert!(unsupported(&io::Error::from_raw_os_error(code)));
        }
        for code in [5, 32, 18, 1117] {
            assert!(!unsupported(&io::Error::from_raw_os_error(code)));
        }
    }
}
