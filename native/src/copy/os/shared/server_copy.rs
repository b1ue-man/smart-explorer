//! One local file copied into a new file by the kernel instead of through
//! this process: Windows `CopyFile2` straight onto the new name (on SMB
//! shares the server copies itself via offload/COPYCHUNK, ReFS clones
//! blocks), elsewhere an exclusively created file filled by
//! `copy_file_range` (server-side on NFS and CIFS). Remote connections that
//! are local filesystems underneath (UNC shares) copy inside one share with
//! it, so no byte crosses the network twice.
use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use super::super::platform;
use super::staging::{open_source, unchanged};

/// Copies the regular local file `source` into the new file `stage` and
/// returns its length. `stage` must not exist (`AlreadyExists`, never
/// adopted); a link, junction or special file as source is refused. The
/// result must be a plain regular file of exactly `expected` bytes from a
/// source that did not change meanwhile, otherwise it is removed again and
/// the copy fails. Nothing is forced to disk: the source stays (a copy, spec
/// decision 1), and the caller publishes the stage.
pub(crate) fn copy_to_new_file(
    source: &Path,
    stage: &Path,
    expected: u64,
    cancel: &AtomicBool,
) -> io::Result<u64> {
    let (reader, before) = open_source(source).map_err(|failure| failure.into_error())?;
    // The kernel copies; `cancel` stops it between its chunks (CopyFile2's
    // cancel flag, the handle loop elsewhere). Progress is counted by the
    // engine once the copy returns.
    let mut quiet = |_: u64| {};
    let (bytes, file) = match platform::copy_by_path(source, stage, cancel, &mut quiet) {
        Some(Ok(Some(copied))) => copied,
        Some(Ok(None)) => {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Serverkopie abgebrochen",
            ))
        }
        Some(Err(error)) => return Err(error),
        None => copy_by_handles(&reader, stage, cancel, &mut quiet)?,
    };
    let complete = bytes == expected
        && file
            .metadata()
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() == expected)
        && unchanged(source, &reader, &before).is_ok();
    if !complete {
        discard_own(stage, file);
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{}: Quelle wurde während der Übertragung geändert",
                source.display()
            ),
        ));
    }
    Ok(bytes)
}

/// The exclusively created `stage`, filled from the open source in the
/// kernel, with the source's permissions.
fn copy_by_handles(
    reader: &File,
    stage: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> io::Result<(u64, File)> {
    let mut writer = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stage)?;
    let copied = platform::copy_handles(reader, &mut writer, cancel, progress).and_then(|copied| {
        let permissions = reader.metadata()?.permissions();
        writer.set_permissions(permissions)?;
        copied.ok_or_else(|| io::Error::new(io::ErrorKind::Interrupted, "Serverkopie abgebrochen"))
    });
    match copied {
        Ok(bytes) => Ok((bytes, writer)),
        Err(error) => {
            discard_own(stage, writer);
            Err(error)
        }
    }
}

/// Removes a copy this call created, only while its name still refers to
/// that very file (never whatever may have replaced it).
fn discard_own(stage: &Path, file: File) {
    let own = platform::file_identity(&file)
        .and_then(|identity| platform::path_matches_identity(stage, identity))
        .unwrap_or(false);
    drop(file);
    if own {
        let _ = std::fs::remove_file(stage);
    }
}

#[cfg(test)]
#[path = "server_copy_tests.rs"]
mod tests;
