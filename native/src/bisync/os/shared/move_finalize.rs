use crate::vfs::{Backend, VfsMeta};
use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

const COMPARE_BUFFER: usize = 256 * 1024;

/// Finish a one-way move only after independently comparing both complete
/// files. This is also the retry path when destination promotion succeeded but
/// the original source deletion failed in an earlier run.
#[allow(clippy::too_many_arguments)]
pub(super) fn verify_and_delete_source(
    source: &dyn Backend,
    source_path: &str,
    destination: &dyn Backend,
    destination_path: &str,
    rel: &str,
    reversible: bool,
    versions_dir: &Path,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let source_meta = regular_file(source.stat(source_path)?, "move source")?;
    let destination_meta = regular_file(destination.stat(destination_path)?, "move destination")?;
    if source_meta.size != destination_meta.size
        || !content_equal(
            source,
            source_path,
            source_meta.id.as_deref(),
            destination,
            destination_path,
            destination_meta.id.as_deref(),
            cancel,
        )?
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "move destination does not exactly match the source; source retained",
        ));
    }
    if reversible {
        super::apply::back_up(source, source_path, rel, versions_dir)?;
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(interrupted());
    }
    let current_source = regular_file(source.stat(source_path)?, "move source")?;
    if !same_identity(&source_meta, &current_source) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "move source changed during verification; source retained",
        ));
    }
    let current_destination =
        regular_file(destination.stat(destination_path)?, "move destination")?;
    if !same_identity(&destination_meta, &current_destination) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "move destination changed during verification; source retained",
        ));
    }
    source.remove_file_id(source_path, source_meta.id.as_deref())
}

fn regular_file(metadata: VfsMeta, label: &str) -> io::Result<VfsMeta> {
    if metadata.is_dir || metadata.is_symlink {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{label} is not a regular file"),
        ));
    }
    Ok(metadata)
}

fn same_identity(before: &VfsMeta, after: &VfsMeta) -> bool {
    before.size == after.size
        && before.mtime_ms == after.mtime_ms
        && match (before.id.as_deref(), after.id.as_deref()) {
            (Some(left), Some(right)) => left == right,
            (None, None) => true,
            _ => false,
        }
}

#[allow(clippy::too_many_arguments)]
fn content_equal(
    left: &dyn Backend,
    left_path: &str,
    left_id: Option<&str>,
    right: &dyn Backend,
    right_path: &str,
    right_id: Option<&str>,
    cancel: &AtomicBool,
) -> io::Result<bool> {
    let mut left = left.open_read_id(left_path, left_id)?;
    let mut right = right.open_read_id(right_path, right_id)?;
    let mut left_buffer = vec![0u8; COMPARE_BUFFER];
    let mut right_buffer = vec![0u8; COMPARE_BUFFER];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(interrupted());
        }
        let left_len = read_chunk(&mut left, &mut left_buffer)?;
        let right_len = read_chunk(&mut right, &mut right_buffer)?;
        if left_len != right_len || left_buffer[..left_len] != right_buffer[..right_len] {
            return Ok(false);
        }
        if left_len == 0 {
            return Ok(true);
        }
    }
}

fn read_chunk(reader: &mut dyn Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0usize;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    Ok(filled)
}

fn interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "move finalization canceled")
}
