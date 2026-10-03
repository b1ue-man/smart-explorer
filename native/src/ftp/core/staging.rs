//! Staged writes on FTP. The protocol has no exclusive create and no
//! no-replace rename, so opening the stage and publishing it are each an
//! absence check right before the one mutating command; the stage itself is a
//! fresh random sibling name (`vfs::unique_staging_path`). The general
//! `rename_no_replace` stays unsupported, so user renames and conflict copies
//! keep refusing. Replacing an existing file is a single RNFR/RNTO pair, which
//! POSIX servers carry out as one `rename(2)`.
//!
//! The checks run on the transfer's own pooled connection, not on the
//! browsing one: at 50 ms one shared connection held every upload to a few
//! files a second, however many ran in parallel. A stage's random name is
//! probed with SIZE (one round trip, no data connection): only a file of that
//! name could be overwritten, a folder makes STOR fail. Publishing lists the
//! parent once for both answers (the stage is still a regular file; nothing
//! has the destination name, or only a regular file when replacing) and
//! renames on the same connection. Without a free pooled connection the
//! publication uses the browsing connection as before, so it never fails for
//! want of a connection.
use std::io;

use super::core_impl::{basename, parent_dir};
use super::io_adapters::FtpConnection;
use crate::vfs::{Backend, VfsMeta};

fn already_exists(path: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("{path} existiert bereits"),
    )
}

/// Fails with `AlreadyExists` when `path` exists; a failed probe is an error,
/// never "absent".
pub(super) fn require_absent<B: Backend + ?Sized>(backend: &B, path: &str) -> io::Result<()> {
    if backend.try_exists(path)? {
        return Err(already_exists(path));
    }
    Ok(())
}

/// Fails with `AlreadyExists` when a file named `path` (a stage's random
/// name) exists: SIZE where the server has it, else a listing of the parent.
pub(super) fn require_stage_free(connection: &FtpConnection, path: &str) -> io::Result<()> {
    let taken = entry(connection, path)?.is_some();
    if taken {
        return Err(already_exists(path));
    }
    Ok(())
}

fn listing(connection: &FtpConnection, folder: &str) -> io::Result<Vec<VfsMeta>> {
    let listing = super::metadata::list(connection, folder)?;
    if !listing.omitted.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "FTP promotion needs a complete, addressable namespace listing",
        ));
    }
    Ok(listing.entries)
}

/// `path`'s entry in a listing of its parent.
fn entry(connection: &FtpConnection, path: &str) -> io::Result<Option<VfsMeta>> {
    match super::metadata::stat(connection, path) {
        Ok(meta) => Ok(Some(meta)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Publishes the flushed stage `staged` as `destination` on `connection`;
/// `replace` accepts an existing regular file there (one RNFR/RNTO).
pub(super) fn publish(
    connection: &FtpConnection,
    staged: &str,
    destination: &str,
    replace: bool,
) -> io::Result<()> {
    let (stage, existing) = if connection.features()?.mlst {
        // Existing objects need one MLST each, not a data connection and
        // full parent listing for every promotion. Ambiguous 550 still uses
        // the metadata module's complete-listing absence proof.
        (entry(connection, staged)?, entry(connection, destination)?)
    } else if parent_dir(staged) == parent_dir(destination) {
        let entries = listing(connection, &parent_dir(destination))?;
        let find = |path: &str| {
            let name = basename(path);
            entries.iter().find(|meta| meta.name == name).cloned()
        };
        (find(staged), find(destination))
    } else {
        (entry(connection, staged)?, entry(connection, destination)?)
    };
    let stage = stage.ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, format!("{staged} nicht gefunden"))
    })?;
    if stage.is_dir || stage.is_symlink || stage.special {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "staged promotion source must be a regular file",
        ));
    }
    match existing {
        None => {}
        Some(_) if !replace => return Err(already_exists(destination)),
        Some(meta) if meta.is_dir || meta.is_symlink || meta.special => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to replace a directory or link-like destination with a file",
            ))
        }
        Some(_) => {}
    }
    super::errors::command_path(staged)?;
    super::errors::command_path(destination)?;
    connection.with_stream_mutation(|stream| {
        stream
            .rename(staged, destination)
            .map_err(super::errors::map)
    })
}
