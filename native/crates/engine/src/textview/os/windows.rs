//! Positioned reads of a local file on Windows.
use std::fs::File;
use std::io;
use std::os::windows::fs::FileExt;

/// Reads at `offset` (`ReadFile` with an explicit offset).
pub(crate) fn read_at(file: &File, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
    file.seek_read(buf, offset)
}
