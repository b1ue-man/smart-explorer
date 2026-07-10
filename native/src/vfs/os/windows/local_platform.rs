use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

fn file_attributes(meta: &std::fs::Metadata) -> u32 {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes()
}

pub(crate) fn local_attrs(meta: &std::fs::Metadata) -> (bool, bool) {
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    let a = file_attributes(meta);
    (
        a & FILE_ATTRIBUTE_HIDDEN != 0,
        a & FILE_ATTRIBUTE_SYSTEM != 0,
    )
}

pub(crate) fn is_reparse_point(meta: &std::fs::Metadata) -> bool {
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    file_attributes(meta) & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

pub(crate) fn to_os(path: &str) -> PathBuf {
    let b = path.as_bytes();
    let rooted;
    let path = if b.len() == 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        rooted = format!("{}/", path);
        rooted.as_str()
    } else {
        path
    };
    PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR))
}

pub(crate) fn remove_file_like(path: &Path) -> std::io::Result<()> {
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    let metadata = std::fs::symlink_metadata(path)?;
    if is_reparse_point(&metadata) && file_attributes(&metadata) & FILE_ATTRIBUTE_DIRECTORY != 0 {
        std::fs::remove_dir(path)
    } else {
        std::fs::remove_file(path)
    }
}

pub(crate) fn rename_no_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // Omitting MOVEFILE_REPLACE_EXISTING is the Win32 no-replace primitive.
    let ok = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}
