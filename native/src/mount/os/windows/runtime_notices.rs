//! Source and license delivery also works for portable and exe-only updates.
use std::{fs::{File, OpenOptions}, io::{self, Read, Write}, os::windows::fs::OpenOptionsExt, path::Path};
use sha2::{Digest, Sha256};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ};
use super::{cache_lease::validate_lock_handle, private_payload::{
    BUNDLED_DOKANY_SOURCE_ARCHIVE, BUNDLED_DOKANY_SOURCE_SHA256,
}};

const NOTICE: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../third-party/dokany/NOTICE.txt"));
const GPL: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../third-party/dokany/LICENSE-GPL-3.0.txt"));
const LGPL: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../third-party/dokany/LICENSE-LGPL-3.0.txt"));

pub(super) fn identity() -> String {
    let mut hash = Sha256::new();
    for bytes in [NOTICE, GPL, LGPL, BUNDLED_DOKANY_SOURCE_SHA256.as_bytes()] {
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    format!("{:x}", hash.finalize())
}

pub(super) fn stage(directory: &Path) -> io::Result<Vec<File>> {
    if BUNDLED_DOKANY_SOURCE_ARCHIVE.is_empty()
        || format!("{:x}", Sha256::digest(BUNDLED_DOKANY_SOURCE_ARCHIVE)) != BUNDLED_DOKANY_SOURCE_SHA256
    {
        return Err(io::Error::other("private runtime corresponding source is missing or invalid"));
    }
    [
        ("NOTICE.txt", NOTICE),
        ("LICENSE-GPL-3.0.txt", GPL),
        ("LICENSE-LGPL-3.0.txt", LGPL),
        ("corresponding-source.zip", BUNDLED_DOKANY_SOURCE_ARCHIVE),
    ].into_iter().map(|(name, bytes)| stage_file(&directory.join(name), bytes)).collect()
}

fn stage_file(path: &Path, bytes: &[u8]) -> io::Result<File> {
    match OpenOptions::new().read(true).write(true).create_new(true)
        .share_mode(FILE_SHARE_READ).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(path) {
        Ok(mut file) => {
            validate_lock_handle(&file)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let mut file = OpenOptions::new().read(true).share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(path)?;
    validate_lock_handle(&file)?;
    if file.metadata()?.len() != bytes.len() as u64 {
        return Err(io::Error::other("private runtime source/notice length differs"));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 { break }
        digest.update(&buffer[..count]);
    }
    if digest.finalize() != Sha256::digest(bytes) {
        return Err(io::Error::other("private runtime source/notice bytes differ"));
    }
    Ok(file)
}
