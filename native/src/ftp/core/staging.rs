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

use super::core_impl::{basename, parent_dir, parse_list_line};
use super::io_adapters::FtpConnection;
use crate::vfs::{Backend, VfsMeta};
use suppaftp::FtpError;

fn io_err<E: std::fmt::Display>(error: E) -> io::Error {
    io::Error::other(error.to_string())
}

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
    let taken = match size_probe(connection, path)? {
        Some(taken) => taken,
        None => entry(connection, path)?.is_some(),
    };
    if taken {
        return Err(already_exists(path));
    }
    Ok(())
}

/// `Some(true)`: SIZE found a file; `Some(false)`: the server answered 550,
/// no file of that name; `None`: the answer says nothing about the name (no
/// SIZE, or not in this transfer mode).
fn size_probe(connection: &FtpConnection, path: &str) -> io::Result<Option<bool>> {
    connection.with_stream_read(|stream| match stream.size(path) {
        Ok(_) => Ok(Some(true)),
        Err(FtpError::UnexpectedResponse(response)) if response.status.code() == 550 => {
            Ok(Some(false))
        }
        Err(FtpError::UnexpectedResponse(_)) => Ok(None),
        Err(error) => Err(io_err(error)),
    })
}

fn listing(connection: &FtpConnection, folder: &str) -> io::Result<Vec<VfsMeta>> {
    let lines = connection.with_stream_read(|stream| stream.list(Some(folder)).map_err(io_err))?;
    lines
        .into_iter()
        .map(|line| parse_list_line(&line))
        .collect()
}

/// `path`'s entry in a listing of its parent.
fn entry(connection: &FtpConnection, path: &str) -> io::Result<Option<VfsMeta>> {
    let name = basename(path);
    Ok(listing(connection, &parent_dir(path))?
        .into_iter()
        .find(|meta| meta.name == name))
}

/// Publishes the flushed stage `staged` as `destination` on `connection`;
/// `replace` accepts an existing regular file there (one RNFR/RNTO).
pub(super) fn publish(
    connection: &FtpConnection,
    staged: &str,
    destination: &str,
    replace: bool,
) -> io::Result<()> {
    let (stage, existing) = if parent_dir(staged) == parent_dir(destination) {
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
    if stage.is_dir || stage.is_symlink {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "staged promotion source must be a regular file",
        ));
    }
    match existing {
        None => {}
        Some(_) if !replace => return Err(already_exists(destination)),
        Some(meta) if meta.is_dir || meta.is_symlink => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to replace a directory or link-like destination with a file",
            ))
        }
        Some(_) => {}
    }
    connection.with_stream_mutation(|stream| stream.rename(staged, destination).map_err(io_err))
}
