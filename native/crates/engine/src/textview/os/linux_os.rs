//! Positioned reads of a local file on Linux and Android.
use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;

/// Reads at `offset` without moving a shared file cursor.
pub(crate) fn read_at(file: &File, offset: u64, buf: &mut [u8]) -> io::Result<usize> {
    file.read_at(buf, offset)
}
