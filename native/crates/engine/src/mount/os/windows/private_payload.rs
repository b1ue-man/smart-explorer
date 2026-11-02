//! Byte-verified private executable state, never part of remote-content eviction.

use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ};

use super::cache_lease::{
    file_information, open_plain_directory, prepare_plain_directory, validate_directory_handle,
    validate_lock_handle,
};

include!(concat!(env!("OUT_DIR"), "/private_dokany.rs"));

pub(super) struct PrivatePayload {
    pub(super) path: PathBuf,
    file: File,
    _directories: Vec<File>,
    _documents: Vec<File>,
}

impl PrivatePayload {
    pub(super) fn stage(cache_root: &Path) -> io::Result<Option<Self>> {
        if BUNDLED_DOKANY_BYTES.is_empty() {
            return Ok(None);
        }
        if BUNDLED_DOKANY_SHA256.len() != 64
            || !BUNDLED_DOKANY_SHA256.bytes().all(|value| value.is_ascii_hexdigit())
            || format!("{:x}", Sha256::digest(BUNDLED_DOKANY_BYTES)) != BUNDLED_DOKANY_SHA256
        {
            return Err(invalid("embedded private Dokany identity is invalid"));
        }
        if !cache_root.is_absolute() {
            return Err(invalid("private runtime requires an absolute audited cache root"));
        }
        // Keep every ancestor spelling stable until FreeLibrary has returned.
        // Final file ownership additionally denies writes, links and deletion.
        let mut directories = cache_root
            .ancestors()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(open_plain_directory)
            .collect::<io::Result<Vec<_>>>()?;
        let runtime_root = cache_root.join("private-runtime");
        directories.push(prepare_plain_directory(&runtime_root)?);
        // License/source updates must not collide with locked files from a
        // previous executable that embeds the same DLL bytes.
        let payload_root = runtime_root.join(format!("{}-{}", BUNDLED_DOKANY_SHA256,
            super::runtime_notices::identity()));
        directories.push(prepare_plain_directory(&payload_root)?);
        // A different basename avoids accidentally selecting this module when
        // the official System32 dokan2.dll is requested in the same process.
        let path = payload_root.join("smart-explorer-dokan2.dll");
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path)
        {
            Ok(mut writer) => {
                validate_lock_handle(&writer)?;
                writer.write_all(BUNDLED_DOKANY_BYTES)?;
                writer.sync_all()?;
                // Image mapping must not retain a writable file handle.
                drop(writer);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let mut file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path)?;
        validate_lock_handle(&file)?;
        if file.metadata()?.len() != BUNDLED_DOKANY_BYTES.len() as u64 {
            return Err(invalid("private Dokany payload length differs from embedded bytes"));
        }
        let mut digest = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        if format!("{:x}", digest.finalize()) != BUNDLED_DOKANY_SHA256 {
            return Err(invalid("private Dokany payload SHA-256 differs from embedded bytes"));
        }
        let documents = super::runtime_notices::stage(&payload_root)?;
        let payload = Self { path, file, _directories: directories, _documents: documents };
        payload.validate_directories()?;
        Ok(Some(payload))
    }

    pub(super) fn validate_directories(&self) -> io::Result<()> {
        // Newly created empty directories can acquire a reparse point through
        // attribute access despite sharing restrictions. Recheck the pinned
        // handles once the locked child chain makes every ancestor nonempty.
        for directory in &self._directories {
            validate_directory_handle(directory)?;
        }
        Ok(())
    }

    /// Confirm the loader's actual module pathname names our locked object.
    /// This supplements (never replaces) the pre-load byte/path ownership.
    pub(super) fn verify_loaded_path(&self, loaded_path: &Path) -> io::Result<()> {
        let loaded = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(loaded_path)?;
        validate_lock_handle(&loaded)?;
        let expected = file_information(&self.file)?;
        let actual = file_information(&loaded)?;
        if (expected.dwVolumeSerialNumber, expected.nFileIndexHigh, expected.nFileIndexLow)
            != (actual.dwVolumeSerialNumber, actual.nFileIndexHigh, actual.nFileIndexLow)
        {
            return Err(invalid("Windows selected a different Dokany module object"));
        }
        Ok(())
    }
}

fn invalid(detail: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, detail)
}
