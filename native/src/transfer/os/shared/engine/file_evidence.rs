//! Observe file content through its backend's actual read contract. The VFS
//! reader owns optical admission; callers never wrap it in a second permit.
use super::{job_store::with_backend, view::Side};
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

#[derive(Clone, Copy)]
enum Observation {
    Published,
    Source,
}

pub(super) fn observe(side: Side<'_>, path: &str, cancel: &AtomicBool) -> io::Result<Proof> {
    observe_as(side, path, None, cancel, Observation::Published)
}

/// Readback is optional for a live acknowledged remote copy. Its absence
/// never supplies a restart ownership proof or authorizes source cleanup.
pub(super) fn observe_available(
    side: Side<'_>,
    path: &str,
    cancel: &AtomicBool,
) -> io::Result<Option<Proof>> {
    match observe(side, path, cancel) {
        Ok(proof) => Ok(Some(proof)),
        Err(error) if readback_unavailable(side, &error) => Ok(None),
        Err(error) => Err(error),
    }
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
) -> io::Result<()> {
    let expected = with_backend(source, |backend| {
        backend.read_size(source_path, source_size)
    })?;
    validate_target_shape(target, path, expected)
}

pub(super) fn validate_target_shape(
    side: Side<'_>,
    path: &str,
    expected_size: Option<u64>,
) -> io::Result<()> {
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
    Ok(())
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
    let current = observe(side, &proof.path, cancel)?;
    if proof.object != current.object
        || proof.id != current.id
        || proof.size != current.size
        || proof.sha256 != current.sha256
    {
        return Err(io::Error::other(format!(
            "Veröffentlichte Datei ist geändert oder durch ein fremdes Objekt ersetzt: {}",
            proof.path
        )));
    }
    Ok(())
}
