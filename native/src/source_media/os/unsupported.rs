use super::Source;
use std::{io, path::Path};
pub(super) fn probe(_path: &Path) -> io::Result<Option<Source>> {
    Ok(None)
}
pub(super) fn probe_regular_file(_path: &Path) -> io::Result<Option<Source>> {
    Ok(None)
}
pub(super) fn try_lock(_file: &std::fs::File, _exclusive: bool) -> io::Result<bool> {
    Ok(true)
}
pub(super) fn unlock(_file: &std::fs::File) -> io::Result<()> {
    Ok(())
}

pub(super) fn probe_file(_file: &std::fs::File) -> io::Result<Option<Source>> {
    Ok(None)
}
