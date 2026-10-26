//! Reopen only descendants of the already selected local root. Child links
//! remain non-recursive boundaries even if changed since the original scan.
use crate::local_access::{DirectoryHandle, LocalEntry};
use std::io;
use std::path::{Component, Path};

pub(crate) fn open_retry_root(base: &str, path: &str) -> io::Result<DirectoryHandle> {
    if !super::retry_contains(base, path) {
        return Err(io::Error::other("retry root is outside selected location"));
    }
    let normalize = |text: &str| {
        let text = text.replace('\\', "/");
        if let Some(unc) = text.strip_prefix("//?/UNC/") {
            format!("//{unc}")
        } else {
            text.strip_prefix("//?/").unwrap_or(&text).to_owned()
        }
    };
    let base = normalize(base);
    let path = normalize(path);
    let base_segments = base.split('/').filter(|p| !p.is_empty()).count();
    let mut current = DirectoryHandle::open_root_consented(Path::new(&base))?;
    for segment in path
        .split('/')
        .filter(|p| !p.is_empty())
        .skip(base_segments)
    {
        if matches!(segment, "." | "..") {
            return Err(io::Error::other("nonliteral retry segment"));
        }
        current = current.open_child(std::ffi::OsStr::new(segment))?;
    }
    Ok(current)
}

pub(super) fn read_confined(
    base: &Path,
    root: &DirectoryHandle,
    path: &Path,
) -> io::Result<Box<dyn Iterator<Item = io::Result<LocalEntry>>>> {
    let relative = path.strip_prefix(base).map_err(io::Error::other)?;
    let mut handle = root.clone();
    for component in relative.components() {
        match component {
            Component::Normal(name) => handle = handle.open_child(name)?,
            _ => return Err(io::Error::other("nonliteral retry descendant")),
        }
    }
    Ok(Box::new(handle.read_directory()?))
}
