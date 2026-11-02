//! Other hosts retain their existing OS/app trash adapters.
use super::{CatalogPage, RestoreOutcome};
use crate::{
    local_access::DirectoryHandle,
    vfs::{RecycleExpectation, RecycleOutcome},
};
use std::{fs::File, io, path::Path};

pub fn available() -> bool {
    false
}
fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "Diese Host-Wiederherstellung ist auf Windows verfügbar",
    )
}
pub fn list(_: Option<&str>) -> io::Result<CatalogPage> {
    Err(unsupported())
}
pub fn restore(_: &str) -> io::Result<RestoreOutcome> {
    Err(unsupported())
}
pub fn recycle_selected(
    _: &Path,
    _: &DirectoryHandle,
    _: &File,
    _: &RecycleExpectation,
) -> io::Result<RecycleOutcome> {
    Err(unsupported())
}
