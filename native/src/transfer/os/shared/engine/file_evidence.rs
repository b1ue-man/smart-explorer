//! Observe file content through its backend's actual read contract. The VFS
//! reader owns optical admission; callers never wrap it in a second permit.
use super::{job_store::with_backend, view::Side};
use crate::vfs::VfsMeta;
use ring::digest::{Context, SHA256};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    io::{self, Read},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};

thread_local! {
    /// The job's meter of bytes read to prove files, while a worker runs one
    /// file (`with_check_meter`): read-backs show in the rate instead of a
    /// frozen display.
    static CHECK_METER: RefCell<Option<Arc<AtomicU64>>> = const { RefCell::new(None) };
}

/// Runs `run` with `meter` counting every byte this thread reads to prove a
/// file.
pub(super) fn with_check_meter<T>(meter: Arc<AtomicU64>, run: impl FnOnce() -> T) -> T {
    let previous = CHECK_METER.with(|cell| cell.replace(Some(meter)));
    let result = run();
    CHECK_METER.with(|cell| *cell.borrow_mut() = previous);
    result
}

fn count_checked(bytes: u64) {
    CHECK_METER.with(|cell| {
        if let Some(meter) = cell.borrow().as_ref() {
            meter.fetch_add(bytes, Ordering::AcqRel);
        }
    });
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Proof {
    pub path: String,
    pub object: Option<String>,
    pub id: Option<String>,
    pub size: u64,
    pub mtime_ms: i64,
    pub sha256: String,
}

/// A file's metadata identity as a host reply already reported it (a
/// stage, a published target or a packet item): the fields of a metadata
/// `Proof`, without a digest. Evidence passed this way replaces the stat
/// and file-identity requests the receipts would otherwise send
/// (`file_recovery::*_known`); it comes from the same authenticated host
/// under the same permission check as those requests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HostEvidence {
    pub path: String,
    pub object: Option<String>,
    pub id: Option<String>,
    pub size: u64,
    pub mtime_ms: i64,
}

impl HostEvidence {
    /// The metadata proof this evidence stands for (empty digest).
    pub(super) fn proof(&self) -> Proof {
        Proof {
            path: self.path.clone(),
            object: self.object.clone(),
            id: self.id.clone(),
            size: self.size,
            mtime_ms: self.mtime_ms,
            sha256: String::new(),
        }
    }
}

#[derive(Clone, Copy)]
enum Observation {
    Published,
    Source,
}

pub(super) fn observe(side: Side<'_>, path: &str, cancel: &AtomicBool) -> io::Result<Proof> {
    observe_as(side, path, None, cancel, Observation::Published)
}

/// The current proof of a published or staged file: hashed on the device
/// that holds it, read back only when that device has no runtime that can
/// hash (the one case where its bytes cross the network, and only in doubt).
pub(super) fn current(side: Side<'_>, path: &str, cancel: &AtomicBool) -> io::Result<Proof> {
    match on_device(side, path, None, cancel)? {
        Some(proof) => Ok(proof),
        None => observe(side, path, cancel),
    }
}

/// `current` for a transfer source: an exported document (no regular size,
/// read through its selected object ID) is always read through its export.
pub(super) fn current_source(
    side: Side<'_>,
    path: &str,
    selected_id: Option<&str>,
    cancel: &AtomicBool,
) -> io::Result<Proof> {
    let exported = with_backend(side, |backend| {
        let meta = backend.stat(path)?;
        backend
            .read_size(path, meta.size)
            .map(|size| size.is_none())
    })?;
    if !exported {
        if let Some(proof) = on_device(side, path, selected_id, cancel)? {
            return Ok(proof);
        }
    }
    observe_source(side, path, selected_id, cancel)
}

/// The file's digest from the device that holds it (`vfs::content_digest`):
/// local disk, a Share peer or room member, the SSH agent. Only the answer
/// crosses the network. `None` when that device cannot hash.
fn on_device(
    side: Side<'_>,
    path: &str,
    selected_id: Option<&str>,
    cancel: &AtomicBool,
) -> io::Result<Option<Proof>> {
    crate::source_media::with_cancellation(cancel, || {
        with_backend(side, |backend| {
            let native = super::super::walk_listers::native(path);
            let medium = if side.is_local() {
                crate::source_media::probe(&native)?
            } else {
                None
            };
            let before = backend.stat(path)?;
            if selected_id.is_some_and(|id| before.id.as_deref() != Some(id)) {
                return Err(io::Error::other(
                    "Gespeicherte Export-/Quellobjekt-ID wurde geändert",
                ));
            }
            if before.is_dir || before.is_symlink || before.special {
                return Err(io::Error::other(format!(
                    "Dateibeleg verweigert Ordner, Link oder Spezialdatei: {path}"
                )));
            }
            let object = crate::vfs::file_identity(backend, path)?;
            let Some(digest) = crate::vfs::content_digest(backend, path, cancel)? else {
                return Ok(None);
            };
            let after = backend.stat(path)?;
            let medium_after = if side.is_local() {
                crate::source_media::probe(&native)?
            } else {
                None
            };
            if medium != medium_after
                || before.id != after.id
                || before.size != after.size
                || before.mtime_ms != after.mtime_ms
                || digest.size != after.size
                || crate::vfs::file_identity(backend, path)? != object
            {
                return Err(io::Error::other(format!(
                    "Datei oder Medium während der Belegprüfung geändert: {path}"
                )));
            }
            Ok(Some(Proof {
                path: path.into(),
                object,
                id: before.id,
                size: digest.size,
                mtime_ms: before.mtime_ms,
                sha256: digest.sha256.to_ascii_lowercase(),
            }))
        })
    })
}

/// The metadata identity of a regular file, without reading it: object
/// identity where the provider has one, ID, size and time. Its empty digest
/// marks it as metadata evidence, never as a content proof.
pub(super) fn stamp(side: Side<'_>, path: &str) -> io::Result<Proof> {
    let meta = with_backend(side, |backend| backend.stat(path))?;
    stamp_of(side, path, meta)
}

/// `stamp` from metadata the caller already has.
pub(super) fn stamp_of(side: Side<'_>, path: &str, meta: VfsMeta) -> io::Result<Proof> {
    if meta.is_dir || meta.is_symlink || meta.special {
        return Err(io::Error::other(format!(
            "Dateibeleg verweigert Ordner, Link oder Spezialdatei: {path}"
        )));
    }
    Ok(Proof {
        path: path.into(),
        object: with_backend(side, |backend| crate::vfs::file_identity(backend, path))?,
        id: meta.id,
        size: meta.size,
        mtime_ms: meta.mtime_ms,
        sha256: String::new(),
    })
}

/// A private stage, recorded before publication so a crash removes exactly
/// it: by the metadata the caller just read, never by content. A local
/// stage is removed only through its object identity, so one without an
/// identity is not recorded.
pub(super) fn stage_record(side: Side<'_>, path: &str, meta: VfsMeta) -> io::Result<Option<Proof>> {
    let stage = stamp_of(side, path, meta)?;
    Ok((!side.is_local() || stage.object.is_some()).then_some(stage))
}

/// Whether the file still has the recorded metadata identity: the same
/// object (where recorded), ID, size and time. A missing file does not.
pub(super) fn stamp_matches(proof: &Proof, side: Side<'_>) -> io::Result<bool> {
    let meta = match with_backend(side, |backend| backend.stat(&proof.path)) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if meta.is_dir
        || meta.is_symlink
        || meta.special
        || meta.id != proof.id
        || meta.size != proof.size
        || meta.mtime_ms != proof.mtime_ms
    {
        return Ok(false);
    }
    Ok(proof.object.is_none()
        || with_backend(side, |backend| {
            crate::vfs::file_identity(backend, &proof.path)
        })? == proof.object)
}

/// `verify` of metadata evidence that returns the stamp it compared: one
/// stat and one identity look instead of a check and a second stamp.
pub(super) fn confirm_stamp(proof: &Proof, side: Side<'_>) -> io::Result<Proof> {
    let changed = || {
        io::Error::other(format!(
            "Datei ist geändert oder durch ein fremdes Objekt ersetzt: {}",
            proof.path
        ))
    };
    if proof.object.is_none() {
        return Err(changed());
    }
    let meta = match with_backend(side, |backend| backend.stat(&proof.path)) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(changed()),
        Err(error) => return Err(error),
    };
    if meta.is_dir || meta.is_symlink || meta.special {
        return Err(changed());
    }
    let now = stamp_of(side, &proof.path, meta)?;
    if now.id != proof.id
        || now.size != proof.size
        || now.mtime_ms != proof.mtime_ms
        || now.object != proof.object
    {
        return Err(changed());
    }
    Ok(now)
}

/// Metadata evidence decides alone only when it names more than a size.
pub(super) fn decisive(proof: &Proof) -> bool {
    proof.object.is_some() || proof.id.is_some() || proof.mtime_ms != 0
}

pub(super) fn readback_unavailable(side: Side<'_>, error: &io::Error) -> bool {
    !side.is_local()
        && matches!(
            error.kind(),
            io::ErrorKind::PermissionDenied | io::ErrorKind::Unsupported
        )
}

pub(super) fn validate_copy_target(
    source: Side<'_>,
    source_path: &str,
    source_size: u64,
    target: Side<'_>,
    path: &str,
) -> io::Result<VfsMeta> {
    let expected = with_backend(source, |backend| {
        backend.read_size(source_path, source_size)
    })?;
    validate_target_shape(target, path, expected)
}

fn validate_target_shape(
    side: Side<'_>,
    path: &str,
    expected_size: Option<u64>,
) -> io::Result<VfsMeta> {
    let meta = with_backend(side, |backend| backend.stat(path))?;
    if meta.is_dir
        || meta.is_symlink
        || meta.special
        || expected_size.is_some_and(|size| meta.size != size)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Uploadziel ist keine reguläre Datei der übertragenen Größe: {path}"),
        ));
    }
    Ok(meta)
}
pub(super) fn observe_source(
    side: Side<'_>,
    path: &str,
    selected_id: Option<&str>,
    cancel: &AtomicBool,
) -> io::Result<Proof> {
    observe_as(side, path, selected_id, cancel, Observation::Source)
}

fn observe_as(
    side: Side<'_>,
    path: &str,
    selected_id: Option<&str>,
    cancel: &AtomicBool,
    kind: Observation,
) -> io::Result<Proof> {
    crate::source_media::with_cancellation(cancel, || {
        with_backend(side, |backend| {
            let native = super::super::walk_listers::native(path);
            let medium = if side.is_local() {
                crate::source_media::probe(&native)?
            } else {
                None
            };
            let before = backend.stat(path)?;
            if selected_id.is_some_and(|id| before.id.as_deref() != Some(id)) {
                return Err(io::Error::other(
                    "Gespeicherte Export-/Quellobjekt-ID wurde geändert",
                ));
            }
            if before.is_dir || before.is_symlink || before.special {
                return Err(io::Error::other(format!(
                    "Dateibeleg verweigert Ordner, Link oder Spezialdatei: {path}"
                )));
            }
            let object = crate::vfs::file_identity(backend, path)?;
            let expected = match kind {
                Observation::Source => backend.read_size(path, before.size)?,
                Observation::Published => Some(before.size),
            };
            // Google-native source entries export through open_read_id; the
            // regular-file API deliberately refuses these source objects.
            let reader = if expected.is_none() {
                let id = selected_id
                    .or(before.id.as_deref())
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| io::Error::other("Exportquelle ohne bestätigte Objekt-ID"))?;
                backend.open_read_id(path, Some(id))?
            } else {
                crate::vfs::open_read_regular(backend, path, before.id.as_deref())?
            };
            let sha256 = hash(reader, expected, cancel)?;
            // hash consumes and drops its reader, releasing optical admission
            // before stat or any subsequent metadata operation acquires again.
            let after = backend.stat(path)?;
            let medium_after = if side.is_local() {
                crate::source_media::probe(&native)?
            } else {
                None
            };
            if medium != medium_after
                || before.id != after.id
                || before.size != after.size
                || before.mtime_ms != after.mtime_ms
                || after.is_dir
                || after.is_symlink
                || after.special
                || crate::vfs::file_identity(backend, path)? != object
            {
                return Err(io::Error::other(format!(
                    "Datei oder Medium während der Belegprüfung geändert: {path}"
                )));
            }
            Ok(Proof {
                path: path.into(),
                object,
                id: before.id,
                size: before.size,
                mtime_ms: before.mtime_ms,
                sha256,
            })
        })
    })
}

fn hash(
    mut reader: Box<dyn Read + Send>,
    expected: Option<u64>,
    cancel: &AtomicBool,
) -> io::Result<String> {
    let mut hash = Context::new(&SHA256);
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        if cancel.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Dateibeleg abgebrochen",
            ));
        }
        let limit = expected.map_or(buffer.len(), |size| {
            size.saturating_sub(total)
                .saturating_add(1)
                .min(buffer.len() as u64) as usize
        });
        match reader.read(&mut buffer[..limit]) {
            Ok(0) => break,
            Ok(read) => {
                total = total.saturating_add(read as u64);
                count_checked(read as u64);
                if expected.is_some_and(|size| total > size) {
                    return Err(io::Error::other("Dateibelegquelle ist gewachsen"));
                }
                hash.update(&buffer[..read]);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    if expected.is_some_and(|size| size != total) {
        return Err(io::Error::other("Dateibelegquelle wurde verkürzt"));
    }
    Ok(hash
        .finish()
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(super) fn verify(
    proof: &Proof,
    side: Side<'_>,
    cancel: &AtomicBool,
    require_object: bool,
) -> io::Result<()> {
    if require_object && proof.object.is_none() {
        return Err(io::Error::other(format!(
            "Dateieigentum ohne stabile Objektidentität ist unbestätigt: {}",
            proof.path
        )));
    }
    if proof.sha256.is_empty() {
        return same_identity(proof, side);
    }
    let now = current(side, &proof.path, cancel)?;
    if proof.object != now.object
        || proof.id != now.id
        || proof.size != now.size
        || proof.sha256 != now.sha256
    {
        return Err(io::Error::other(format!(
            "Veröffentlichte Datei ist geändert oder durch ein fremdes Objekt ersetzt: {}",
            proof.path
        )));
    }
    Ok(())
}

/// Metadata evidence (`stamp`): the same object, ID, size and time.
fn same_identity(proof: &Proof, side: Side<'_>) -> io::Result<()> {
    if proof.object.is_none() || !stamp_matches(proof, side)? {
        return Err(io::Error::other(format!(
            "Datei ist geändert oder durch ein fremdes Objekt ersetzt: {}",
            proof.path
        )));
    }
    Ok(())
}
