//! The se program and each renameable ancestor held open from the elevation
//! request until the helper runs, so neither can be replaced or renamed in
//! between; its SHA-256 travels to the helper, which compares its own
//! (same pattern as `core/src/local_access/os/windows/image_lock.rs`, which
//! that crate keeps private).
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read},
    os::windows::fs::{MetadataExt, OpenOptionsExt},
    path::PathBuf,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_FLAG_SEQUENTIAL_SCAN, FILE_SHARE_READ,
};

pub(super) struct LockedImage {
    pub(super) path: PathBuf,
    pub(super) hash: String,
    _file: File,
    _parents: Vec<File>,
}

impl LockedImage {
    pub(super) fn at(path: PathBuf) -> io::Result<Self> {
        if !path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::CurDir | std::path::Component::ParentDir
                )
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Programm-Pfad ist nicht absolut und normalisiert",
            ));
        }
        let mut parents = Vec::new();
        let mut chain: Vec<_> = path
            .parent()
            .ok_or_else(|| io::Error::other("Programm ohne Elternpfad"))?
            .ancestors()
            .collect();
        chain.reverse();
        for parent in chain {
            if parent.parent().is_none() {
                continue;
            }
            let directory = OpenOptions::new()
                .access_mode(0)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
                .open(parent)?;
            let metadata = directory.metadata()?;
            if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                return Err(io::Error::other(
                    "Programm-Pfad durchquert eine Verzeichnisverknüpfung",
                ));
            }
            parents.push(directory);
        }
        let mut file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_SEQUENTIAL_SCAN)
            .open(&path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::other(
                "Programm ist keine direkte reguläre Datei",
            ));
        }
        let mut hash = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        Ok(Self {
            path,
            hash: format!("{:x}", hash.finalize()),
            _file: file,
            _parents: parents,
        })
    }
}
