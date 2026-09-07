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
    read_directory_with_layout(path, Layout::Extended)
}

fn read_directory_with_layout(path: &Path, layout: Layout) -> io::Result<Directory> {
    read_directory_with_query(path, layout, query_directory)
}

fn read_directory_with_query(
    path: &Path,
    layout: Layout,
    query: impl FnMut(&File, FILE_INFO_BY_HANDLE_CLASS, &mut [u64]) -> io::Result<()>,
) -> io::Result<Directory> {
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
    let mut directory = Directory {
        path: path.to_path_buf(),
        file,
        buffer: vec![0; BUFFER_WORDS],
        cursor: None,
        layout,
        started: false,
        ended: false,
        fallback: None,
    };
    // Establish the listing here so callers can distinguish a failed root
    // from an individual entry that fails after enumeration has begun.
    if !directory.refill_with(query)? {
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

impl Directory {
    fn refill(&mut self) -> io::Result<bool> {
        self.refill_with(query_directory)
    }

    fn refill_with(
        &mut self,
        mut query: impl FnMut(&File, FILE_INFO_BY_HANDLE_CLASS, &mut [u64]) -> io::Result<()>,
    ) -> io::Result<bool> {
        loop {
            self.buffer.fill(0);
            let class = match (self.layout, self.started) {
                (Layout::Extended, false) => FileIdExtdDirectoryRestartInfo,
                (Layout::Extended, true) => FileIdExtdDirectoryInfo,
                (Layout::Full, false) => FileFullDirectoryRestartInfo,
                (Layout::Full, true) => FileFullDirectoryInfo,
            };
            let error = match query(&self.file, class, &mut self.buffer) {
                Ok(()) => {
                    self.started = true;
                    self.cursor = Some(0);
                    return Ok(true);
                }
                Err(error) => error,
            };
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
    fn analytics_access_task_automatic_query_fallback_preserves_access_denial() {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::write(fixture.path().join("file"), vec![0; 11]).unwrap();
        std::fs::create_dir(fixture.path().join("sub")).unwrap();
        let assert_contents = |directory: Directory| {
            let entries: Vec<_> = directory.collect::<io::Result<_>>().unwrap();
            assert_eq!(entries.len(), 2);
            let file = entries.iter().find(|entry| entry.name == "file").unwrap();
            assert_eq!(file.kind, EntryKind::File);
            assert_eq!(file.size, 11);
            assert_eq!(
                entries
                    .iter()
                    .find(|entry| entry.name == "sub")
                    .unwrap()
                    .kind,
                EntryKind::Directory
            );
        };

        let mut calls = Vec::new();
        let full =
            read_directory_with_query(fixture.path(), Layout::Extended, |file, class, buffer| {
                calls.push(class);
                if class == FileIdExtdDirectoryRestartInfo {
                    Err(io::Error::from_raw_os_error(87))
                } else {
                    query_directory(file, class, buffer)
                }
            })
            .unwrap();
        assert_eq!(
            calls,
            [FileIdExtdDirectoryRestartInfo, FileFullDirectoryRestartInfo]
        );
        assert!(matches!(full.layout, Layout::Full));
        assert!(full.fallback.is_none());
        assert_contents(full);

        let mut calls = Vec::new();
        let ordinary =
            read_directory_with_query(fixture.path(), Layout::Extended, |_, class, _| {
                calls.push(class);
                Err(io::Error::from_raw_os_error(50))
            })
            .unwrap();
        assert_eq!(
            calls,
            [FileIdExtdDirectoryRestartInfo, FileFullDirectoryRestartInfo]
        );
        assert!(ordinary.fallback.is_some());
        assert_contents(ordinary);

        // A denial at either query boundary is not an unsupported-class signal.
        // The ordinary listing above proves a wrong fallback would hide it.
        for deny_full in [false, true] {
            let mut calls = Vec::new();
            let result =
                read_directory_with_query(fixture.path(), Layout::Extended, |_, class, _| {
                    calls.push(class);
                    let code = if deny_full && class == FileIdExtdDirectoryRestartInfo {
                        87
                    } else {
                        5
                    };
                    Err(io::Error::from_raw_os_error(code))
                });
            let error = result
                .err()
                .expect("query denial must fail directory startup");
            assert_eq!(error.raw_os_error(), Some(5));
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            let expected = if deny_full {
                vec![FileIdExtdDirectoryRestartInfo, FileFullDirectoryRestartInfo]
            } else {
                vec![FileIdExtdDirectoryRestartInfo]
            };
            assert_eq!(calls, expected);
        }
    }

    #[test]
    fn analytics_access_task_full_record_fallback_and_reparse_classification() {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::write(fixture.path().join("file"), vec![0; 11]).unwrap();
        std::fs::create_dir(fixture.path().join("sub")).unwrap();
        let full = read_directory_with_layout(fixture.path(), Layout::Full).unwrap();
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
