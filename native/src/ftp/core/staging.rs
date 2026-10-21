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
//! files a second, however many ran in parallel. A stage's name is probed
//! with SIZE (one round trip, no data connection) unless it is one of the
//! engine's `<file>.se-upload-<16 hex>` names made from the system's random
//! source: nothing can exist under 64 random bits before they are drawn
//! (security answer S6, a). Only a file of that name could be overwritten, a
//! folder makes STOR fail. Publishing lists the
//! parent once for both answers (the stage is still a regular file; nothing
//! has the destination name, or only a regular file when replacing) and
//! renames on the same connection. Without a free pooled connection the
//! publication uses the browsing connection as before, so it never fails for
//! want of a connection.
use std::io;

use super::core_impl::{basename, parent_dir};
use super::io_adapters::FtpConnection;
use crate::vfs::{Backend, VfsMeta};
use suppaftp::{RustlsFtpStream, Status};

/// Whether every stage name is probed before STOR (`require_stage_free`).
/// `false`: only names that do not carry 64 bits from the system's random
/// source are probed (`from_system_random`). Security answer S6 (a): a
/// foreign object would have to exist under a name that was drawn only now.
const FTP_PROBE_STAGE_NAME: bool = false;

/// Marker of the engine's stage names (`engine::publish::stage_name`).
const STAGE_MARKER: &str = ".se-upload-";

/// Whether the name ends in `.se-upload-` and sixteen lowercase hex digits.
/// `engine::random_hex` writes exactly those when the system's random source
/// answered; its stand-in value starts with `w` and does not match.
pub(super) fn from_system_random(path: &str) -> bool {
    path.rsplit_once(STAGE_MARKER).is_some_and(|(_, tail)| {
        tail.len() == 16
            && tail
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// Whether the stage `path` is looked at before STOR.
pub(super) fn probe_needed(probe_all: bool, path: &str) -> bool {
    probe_all || !from_system_random(path)
}

/// Whether `publish` looks at the stage (MLST or a listing) before RNFR. When
/// `false`, the 550 of RNFR is the answer for a missing stage (`NotFound`, as
/// before); the destination is still looked at, it is the check against
/// replacing. Stays `true` (security answer S6, b): without the look a
/// swapped object (folder, link) would be published under the destination
/// name and reported as the data this client wrote.
const FTP_VERIFY_STAGE_BEFORE_RENAME: bool = true;

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

/// Fails with `AlreadyExists` when a file named `path` (a stage's name)
/// exists: SIZE where the server has it, else a listing of the parent. A name
/// drawn from the system's random source is not looked at.
pub(super) fn require_stage_free(connection: &FtpConnection, path: &str) -> io::Result<()> {
    if !probe_needed(FTP_PROBE_STAGE_NAME, path) {
        return Ok(());
    }
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
    publish_with(
        FTP_VERIFY_STAGE_BEFORE_RENAME,
        connection,
        staged,
        destination,
        replace,
    )
}

/// `publish` with the stage look-up (`verify`) chosen by the caller, so both
/// branches of `FTP_VERIFY_STAGE_BEFORE_RENAME` can be exercised.
pub(super) fn publish_with(
    verify: bool,
    connection: &FtpConnection,
    staged: &str,
    destination: &str,
    replace: bool,
) -> io::Result<()> {
    let (stage, existing) = if !verify {
        (None, entry(connection, destination)?)
    } else if connection.features()?.mlst {
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
    if verify {
        let stage = stage.ok_or_else(|| not_found(staged))?;
        if stage.is_dir || stage.is_symlink || stage.special {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "staged promotion source must be a regular file",
            ));
        }
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
        if verify {
            stream
                .rename(staged, destination)
                .map_err(super::errors::map)
        } else {
            rename_unverified(stream, staged, destination)
        }
    })
}

fn not_found(path: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("{path} nicht gefunden"))
}

/// RNFR and RNTO as `rename` sends them, but told apart: the 550 of RNFR
/// means the stage is missing (`NotFound`), whatever RNTO answers is a
/// failure of the target.
fn rename_unverified(
    stream: &mut RustlsFtpStream,
    staged: &str,
    destination: &str,
) -> io::Result<()> {
    match stream.custom_command(format!("RNFR {staged}"), &[Status::RequestFilePending]) {
        Ok(_) => {}
        Err(error) if super::errors::reply_code(&error) == Some(550) => {
            return Err(not_found(staged))
        }
        Err(error) => return Err(super::errors::map(error)),
    }
    let expected = [Status::RequestedFileActionOk, Status::CommandOk];
    stream
        .custom_command(format!("RNTO {destination}"), &expected)
        .map(|_| ())
        .map_err(super::errors::map)
}
