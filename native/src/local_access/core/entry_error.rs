//! Failures of single directory entries that still name the entry.
use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::{fmt, io};

/// The metadata or type of one directory entry could not be read. The
/// entry's name travels inside the `io::Error` (original kind kept), so a
/// tolerant listing can leave out just this entry; the message stays
/// `<entry path>: <error>`, which callers match on.
#[derive(Debug)]
pub(crate) struct EntryError {
    name: OsString,
    message: String,
}

impl EntryError {
    /// `error` of the entry `name` found at `path`.
    pub(crate) fn wrap(name: OsString, path: &Path, error: io::Error) -> io::Error {
        let message = format!("{}: {error}", path.display());
        io::Error::new(error.kind(), Self { name, message })
    }

    /// The entry an error of a directory listing names, if any.
    pub(crate) fn name_of(error: &io::Error) -> Option<&OsStr> {
        error
            .get_ref()?
            .downcast_ref::<Self>()
            .map(|entry| entry.name.as_os_str())
    }
}

impl fmt::Display for EntryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for EntryError {}
