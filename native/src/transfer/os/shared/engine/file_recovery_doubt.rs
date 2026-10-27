//! In-doubt ownership of a published file whose receipt carries no stored
//! content proof (the live path records none since 2026-10-27). Decided only
//! when a job is resumed or retried, never during the live transfer: kind and
//! size first, then the content digest of source and target, each hashed on
//! the device that holds it. Bytes cross the network only for a side whose
//! device cannot hash (read-back of that side alone).
use std::io;
use std::sync::atomic::AtomicBool;

use super::super::file_evidence::{current, current_source, readback_unavailable, Proof};
use super::{snapshot, with_backend, Receipt, Side};

/// What the destination recorded in a receipt holds now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ownership {
    /// The source's content: this job's own publication (or an identical
    /// file, which is just as good). Skipped; a move may clean its source.
    Own,
    /// Something else: never adopted, the job's conflict policy decides.
    Foreign,
    /// Nothing: the file is transferred again.
    Absent,
}

/// The decision that needs no file content: `Some` when the destination is
/// absent, not a regular file, or of another size than the source; `None`
/// when only the content can decide.
pub(super) fn shape(
    receipt: &Receipt,
    source: Side<'_>,
    target: Side<'_>,
) -> io::Result<Option<Ownership>> {
    let meta = match with_backend(target, |backend| backend.stat(&receipt.destination)) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Some(Ownership::Absent))
        }
        Err(error) => return Err(error),
    };
    if meta.is_dir || meta.is_symlink || meta.special {
        return Ok(Some(Ownership::Foreign));
    }
    // The source as it is now (a moved source that is gone: as recorded).
    // An exported document has no regular size; its content decides.
    let expected = match with_backend(source, |backend| {
        let meta = backend.stat(&receipt.source.path)?;
        backend.read_size(&receipt.source.path, meta.size)
    }) {
        Ok(size) => size,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Some(receipt.source.size),
        Err(error) => return Err(error),
    };
    Ok(expected
        .filter(|size| *size != meta.size)
        .map(|_| Ownership::Foreign))
}

/// The full in-doubt decision for a receipt whose source still exists: the
/// target is this job's own when it holds the source's current content (a
/// source edited since then that the target already matches needs nothing
/// either). An error whose kind `readback_unavailable` accepts means neither
/// device can hash nor be read: the content stays undecided.
pub(super) fn ownership(
    receipt: &Receipt,
    source: Side<'_>,
    target: Side<'_>,
    cancel: &AtomicBool,
) -> io::Result<Ownership> {
    match content(receipt, source, target, cancel)? {
        Some((decided, _)) => Ok(decided),
        None => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Quelle des Dateibelegs fehlt: {}", receipt.source.path),
        )),
    }
}

/// `ownership` against the source, or a move's quarantined source while
/// the source name is empty, with the target's hashed proof when its
/// content decided. `None` when neither exists any more.
pub(super) fn content(
    receipt: &Receipt,
    source: Side<'_>,
    target: Side<'_>,
    cancel: &AtomicBool,
) -> io::Result<Option<(Ownership, Option<Proof>)>> {
    let Some((path, id)) = reference(receipt, source)? else {
        return Ok(None);
    };
    if let Some(decided) = shape(receipt, source, target)? {
        return Ok(Some((decided, None)));
    }
    let ours = undecided(source, &path, current_source(source, &path, id, cancel))?;
    let theirs = undecided(
        target,
        &receipt.destination,
        current(target, &receipt.destination, cancel),
    )?;
    let decided = if same_content(&ours, &theirs) {
        Ownership::Own
    } else {
        Ownership::Foreign
    };
    Ok(Some((decided, Some(theirs))))
}

/// The file whose content the target must hold, with its selected ID.
fn reference<'r>(
    receipt: &'r Receipt,
    source: Side<'_>,
) -> io::Result<Option<(String, Option<&'r str>)>> {
    let candidates = [
        Some((&receipt.source.path, receipt.source.id.as_deref())),
        receipt
            .quarantine
            .as_ref()
            .map(|quarantine| (&quarantine.path, None)),
    ];
    for (path, id) in candidates.into_iter().flatten() {
        match with_backend(source, |backend| backend.stat(path)) {
            Ok(_) => return Ok(Some((path.clone(), id))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

fn same_content(ours: &Proof, theirs: &Proof) -> bool {
    ours.size == theirs.size && ours.sha256.eq_ignore_ascii_case(&theirs.sha256)
}

/// Names why the content stays undecided, keeping the error's kind.
fn undecided(side: Side<'_>, path: &str, result: io::Result<Proof>) -> io::Result<Proof> {
    result.map_err(|error| {
        if readback_unavailable(side, &error) {
            io::Error::new(
                error.kind(),
                format!(
                    "Inhalt von „{path}“ kann auf dem Gerät weder geprüft noch zurückgelesen werden ({error}); ob die Datei zu diesem Auftrag gehört, bleibt unbestätigt"
                ),
            )
        } else {
            error
        }
    })
}

/// The source still is the file that was sent: identity, size and time
/// from its metadata, without reading its content.
pub(super) fn source_unchanged(receipt: &Receipt, side: Side<'_>) -> io::Result<()> {
    let now = snapshot(side, &receipt.source.path);
    if !now.verified
        || now.directory
        || now.link
        || now.id != receipt.source.id
        || now.object != receipt.source.object
        || now.size != receipt.source.size
        || now.mtime_ms != receipt.source.mtime_ms
    {
        return Err(io::Error::other(format!(
            "Quelle des Dateibelegs wurde geändert: {}",
            receipt.source.path
        )));
    }
    Ok(())
}
