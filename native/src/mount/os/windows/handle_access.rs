use std::io;

const FILE_READ_DATA: u32 = 0x0000_0001;
const FILE_WRITE_DATA: u32 = 0x0000_0002;
const FILE_APPEND_DATA: u32 = 0x0000_0004;
const FILE_EXECUTE: u32 = 0x0000_0020;
const DELETE_ACCESS: u32 = 0x0001_0000;
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const GENERIC_ALL: u32 = 0x1000_0000;
const GENERIC_EXECUTE: u32 = 0x2000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const GENERIC_READ: u32 = 0x8000_0000;
const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_SHARE_WRITE: u32 = 0x0000_0002;
pub(super) const FILE_SHARE_DELETE: u32 = 0x0000_0004;

pub(super) fn share_allows(share_access: u32, desired_access: u32) -> bool {
    (!requests_read(desired_access) || share_access & FILE_SHARE_READ != 0)
        && (!requests_write(desired_access) || share_access & FILE_SHARE_WRITE != 0)
        && (!requests_delete(desired_access) || share_access & FILE_SHARE_DELETE != 0)
}

pub(super) fn require_delete_access(access: u32) -> io::Result<()> {
    if requests_delete(access) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "file handle has no delete access",
        ))
    }
}

pub(super) fn same_or_descendant(path: &str, ancestor: &str) -> bool {
    path == ancestor
        || path
            .strip_prefix(ancestor.trim_end_matches('\\'))
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

pub(super) fn callback_path_key(path: &str, case_sensitive: bool) -> String {
    let normalized = path.replace('/', "\\");
    let trimmed = normalized.trim_end_matches('\\');
    let normalized = if trimmed.is_empty() && !normalized.is_empty() {
        "\\".to_string()
    } else {
        trimmed.to_string()
    };
    if case_sensitive {
        normalized
    } else {
        crate::mount::windows_ordinal_key(&normalized)
    }
}

pub(super) fn sharing_violation(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::WouldBlock, message)
}

pub(super) fn invalid_handle(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, message)
}

fn requests_read(access: u32) -> bool {
    access
        & (FILE_READ_DATA
            | FILE_EXECUTE
            | MAXIMUM_ALLOWED
            | GENERIC_READ
            | GENERIC_EXECUTE
            | GENERIC_ALL)
        != 0
}

fn requests_write(access: u32) -> bool {
    access & (FILE_WRITE_DATA | FILE_APPEND_DATA | MAXIMUM_ALLOWED | GENERIC_WRITE | GENERIC_ALL)
        != 0
}

fn requests_delete(access: u32) -> bool {
    access & (DELETE_ACCESS | MAXIMUM_ALLOWED | GENERIC_ALL) != 0
}
