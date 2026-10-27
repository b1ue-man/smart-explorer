//! Publication evidence is tied to one opened regular file and its held parent.
use crate::local_access::ConfinedDirectory;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    ffi::OsString,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

/// Reconfirm a journaled destination before destroying its retained source.
/// Successful hashing alone is insufficient: both file and actual parent
/// flush must succeed, and the original parent/name must still bind the file.
pub(crate) fn confirm_transfer_publication(
    target: &Path,
    expected_identity: &str,
    expected_sha256: &str,
    cancel: &AtomicBool,
) -> io::Result<()> {
    confirm_saved_publication(
        target,
        expected_identity,
        Some(expected_sha256),
        cancel,
        SavedPublication::Unconfirmed,
    )
}

/// A durably acknowledged publication already records its successful platform
/// operation. Revalidate that same object and content before retained-source
/// cleanup; do not turn the receipt into a general directory-flush capability.
pub(crate) fn confirm_acknowledged_transfer_publication(
    target: &Path,
    expected_identity: &str,
    expected_sha256: &str,
    cancel: &AtomicBool,
) -> io::Result<()> {
    confirm_saved_publication(
        target,
        expected_identity,
        Some(expected_sha256),
        cancel,
        SavedPublication::Acknowledged,
    )
}

/// `confirm_transfer_publication` (or, for a durably acknowledged receipt,
/// `confirm_acknowledged_transfer_publication`) bound by the object identity
/// alone: for a receipt whose unchanged metadata identity decided, so the
/// destination is flushed and bound without reading its content.
pub(crate) fn confirm_transfer_publication_identity(
    target: &Path,
    expected_identity: &str,
    acknowledged: bool,
    cancel: &AtomicBool,
) -> io::Result<()> {
    confirm_saved_publication(
        target,
        expected_identity,
        None,
        cancel,
        if acknowledged {
            SavedPublication::Acknowledged
        } else {
            SavedPublication::Unconfirmed
        },
    )
}

enum SavedPublication {
    Unconfirmed,
    Acknowledged,
}

fn confirm_saved_publication(
    target: &Path,
    expected_identity: &str,
    expected_sha256: Option<&str>,
    cancel: &AtomicBool,
    basis: SavedPublication,
) -> io::Result<()> {
    if expected_identity.is_empty()
        || expected_sha256.is_some_and(|sha256| {
            sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "publication receipt lacks complete identity/SHA-256 evidence",
        ));
    }
    active(cancel)?;
    let mut held = Publication::open(target)?;
    held.require_identity(expected_identity)?;
    if let Some(sha256) = expected_sha256 {
        held.require_digest(sha256, cancel)?;
    }
    active(cancel)?;
    held.file.sync_all()?;
    active(cancel)?;
    match basis {
        SavedPublication::Acknowledged => {
            super::platform::confirm_live_publication_namespace(&held.parent, target)?;
        }
        SavedPublication::Unconfirmed => {
            super::platform::confirm_publication_namespace(&held.parent)?;
        }
    }
    // Re-read the same handle after flushing. This also detects changed bytes
    // during the first read/flush instead of accepting an earlier cached hash.
    if let Some(sha256) = expected_sha256 {
        held.require_digest(sha256, cancel)?;
    }
    held.require_identity(expected_identity)?;
    held.verify_binding()?;
    if super::imp::local_file_identity_handle(&held.parent.entry(&held.name)?)? != expected_identity
    {
        return Err(changed());
    }
    active(cancel)
}

/// The normal publication path retains its own flush-capable stage handle.
/// Hashing uses a readable handle bound to that same still-live object, so a
/// write-only stage and a final readonly attribute both remain supported.
/// The caller has received success from the platform's publication primitive.
pub(crate) fn confirm_transfer_publication_handle(
    target: &Path,
    file: &File,
    expected_identity: &str,
    expected_sha256: &str,
    cancel: &AtomicBool,
) -> io::Result<()> {
    if expected_identity.is_empty()
        || expected_sha256.len() != 64
        || !expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "publication receipt lacks complete identity/SHA-256 evidence",
        ));
    }
    active(cancel)?;
    let mut held = Publication::from_live(target, file)?;
    held.require_identity(expected_identity)?;
    held.require_digest(expected_sha256, cancel)?;
    active(cancel)?;
    file.sync_all()?;
    active(cancel)?;
    // Windows has already returned from this exact WRITE_THROUGH move; a
    // general directory flush is neither its proof nor a new prerequisite.
    // Platforms needing a parent fsync retain their actual held-parent proof.
    super::platform::confirm_live_publication_namespace(&held.parent, target)?;
    held.require_digest(expected_sha256, cancel)?;
    held.require_identity(expected_identity)?;
    held.verify_binding()?;
    if super::imp::local_file_identity_handle(&held.parent.entry(&held.name)?)? != expected_identity
    {
        return Err(changed());
    }
    active(cancel)
}

/// Acquire flush capability before attempting a direct rename. An unavailable
/// handle selects the existing stage path before any publication takes place.
pub(super) fn open_live_move_source(source: &Path, expected: &File) -> io::Result<File> {
    let name = source
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(changed)?;
    let parent_path = source
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = ConfinedDirectory::open_root(parent_path)?;
    let file = super::platform::open_move_publication_file(&parent, name)?;
    if !super::platform::same_file_object(&file, expected)? {
        return Err(changed());
    }
    Ok(file)
}

/// Detect a cross-mount/volume move before calling any rename primitive.
/// This yields a genuine unpublished EXDEV even for a FUSE/overlay target;
/// an EXDEV returned after dispatch remains an ordinary unconfirmed error.
pub(super) fn preflight_direct_move(source: &File, target: &Path) -> io::Result<()> {
    let result = (|| {
        let parent_path = target
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = ConfinedDirectory::open_root(parent_path)?;
        if super::platform::different_rename_boundary(source, parent.file())? {
            return Err(io::Error::new(
                io::ErrorKind::CrossesDevices,
                "direct move crosses an observed mount/volume boundary before mutation",
            ));
        }
        Ok(())
    })();
    result.map_err(|error| crate::vfs::unpublished_error(error.kind(), error.to_string()))
}

/// An error return alone is insufficient on NFS or an arbitrary provider.
/// Before revoking a successful preparation, prove that the actual stage
/// still names our live object and the destination does not name that object.
pub(super) fn confirm_unpublished(
    staged: &Path,
    target: &Path,
    file: &File,
    cause: &io::Error,
) -> io::Result<()> {
    let stage_name = staged.file_name().ok_or_else(changed)?;
    let target_name = target.file_name().ok_or_else(changed)?;
    let parent_path = |path: &Path| {
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    };
    let stage_path = parent_path(staged);
    let target_path = parent_path(target);
    let stage_parent = ConfinedDirectory::open_root(&stage_path)?;
    let before_mutation = crate::vfs::unpublished_of(cause);
    let actual = stage_parent.entry(stage_name)?;
    let class = crate::local_access::classify_open_file(&actual)?;
    if !actual.metadata()?.is_file()
        || class.link_like
        || class.special
        || !super::platform::same_file_object(&actual, file)?
    {
        return Err(changed());
    }
    // A typed pre-dispatch refusal is independent of any remote namespace
    // observation: this execution never called its mutation primitive.
    if before_mutation {
        return if ConfinedDirectory::open_root(&stage_path)?.identity()?
            == stage_parent.identity()?
            && super::platform::same_file_object(&stage_parent.entry(stage_name)?, file)?
        {
            Ok(())
        } else {
            Err(changed())
        };
    }
    let target_parent = ConfinedDirectory::open_root(&target_path)?;
    super::platform::require_local_publication_observation(stage_parent.file())?;
    super::platform::require_local_publication_observation(target_parent.file())?;
    match target_parent.entry(target_name) {
        Ok(target) if super::platform::same_file_object(&target, file)? => {
            return Err(io::Error::other(
                "destination names the staged object; publication outcome remains unconfirmed",
            ))
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    if ConfinedDirectory::open_root(&stage_path)?.identity()? != stage_parent.identity()?
        || ConfinedDirectory::open_root(&target_path)?.identity()? != target_parent.identity()?
        || !super::platform::same_file_object(&stage_parent.entry(stage_name)?, file)?
    {
        return Err(changed());
    }
    Ok(())
}

/// Preserve ordinary no-replace conflicts on every supported filesystem.
/// This check never mutates: its errors may be explicitly marked unpublished
/// even when a later remote rename error would have an ambiguous outcome.
pub(super) fn preflight_no_replace(target: &Path) -> io::Result<()> {
    match crate::local_access::symlink_metadata(target) {
        Ok(_) => Err(crate::vfs::unpublished_error(
            io::ErrorKind::AlreadyExists,
            format!(
                "no-replace destination already exists before mutation: {}",
                target.display()
            ),
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(crate::vfs::unpublished_error(
            error.kind(),
            format!(
                "no-replace destination check failed before mutation at {}: {error}",
                target.display()
            ),
        )),
    }
}

/// An immediately published stage keeps its real live file handle. It can
/// prove and flush that object without requiring any persisted-ID capability
/// or reopening a read-only destination for newly requested write access.
/// This follows a successfully acknowledged platform publication operation.
pub(super) fn confirm_handle_publication(
    target: &Path,
    file: &File,
    cancel: &AtomicBool,
) -> io::Result<()> {
    active(cancel)?;
    let name = target
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(changed)?;
    let parent_path = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = ConfinedDirectory::open_root(parent_path)?;
    let reached = parent.entry(name)?;
    if !super::platform::same_file_object(&reached, file)? {
        return Err(changed());
    }
    file.sync_all()?;
    active(cancel)?;
    super::platform::confirm_live_publication_namespace(&parent, target)?;
    let current = ConfinedDirectory::open_root(parent_path)?;
    let reached = parent.entry(name)?;
    let class = crate::local_access::classify_open_file(&reached)?;
    if current.identity()? != parent.identity()?
        || !reached.metadata()?.is_file()
        || class.link_like
        || class.special
        || !super::platform::same_file_object(&reached, file)?
    {
        return Err(changed());
    }
    active(cancel)
}

struct Publication {
    parent: ConfinedDirectory,
    parent_path: PathBuf,
    name: OsString,
    file: File,
}

impl Publication {
    fn open(target: &Path) -> io::Result<Self> {
        let name = target
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "publication target requires one child name",
                )
            })?
            .to_owned();
        let parent_path = target
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let parent = ConfinedDirectory::open_root(&parent_path)?;
        let file = super::platform::open_publication_file(&parent, &name)?;
        let class = crate::local_access::classify_open_file(&file)?;
        if !file.metadata()?.is_file() || class.link_like || class.special {
            return Err(changed());
        }
        Ok(Self {
            parent,
            parent_path,
            name,
            file,
        })
    }

    fn from_live(target: &Path, live: &File) -> io::Result<Self> {
        let name = target
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or_else(changed)?
            .to_owned();
        let parent_path = target
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        let parent = ConfinedDirectory::open_root(&parent_path)?;
        let file = parent.open_file(&name, false, false, false, false)?;
        let class = crate::local_access::classify_open_file(&file)?;
        let live_class = crate::local_access::classify_open_file(live)?;
        if !file.metadata()?.is_file()
            || class.link_like
            || class.special
            || !live.metadata()?.is_file()
            || live_class.link_like
            || live_class.special
            || !super::platform::same_file_object(&file, live)?
        {
            return Err(changed());
        }
        Ok(Self {
            parent,
            parent_path,
            name,
            file,
        })
    }

    fn require_identity(&self, expected: &str) -> io::Result<()> {
        if super::imp::local_file_identity_handle(&self.file)? == expected {
            Ok(())
        } else {
            Err(changed())
        }
    }

    fn require_digest(&mut self, expected: &str, cancel: &AtomicBool) -> io::Result<()> {
        active(cancel)?;
        self.file.seek(SeekFrom::Start(0))?;
        let before = self.file.metadata()?;
        let mut digest = Sha256::new();
        let mut buffer = vec![0u8; 256 * 1024];
        let mut length = 0u64;
        loop {
            active(cancel)?;
            let read = match self.file.read(&mut buffer) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            active(cancel)?;
            if read == 0 {
                break;
            }
            length = length.checked_add(read as u64).ok_or_else(changed)?;
            digest.update(&buffer[..read]);
        }
        let after = self.file.metadata()?;
        let actual = format!("{:x}", digest.finalize());
        if length != before.len()
            || length != after.len()
            || before.modified().ok() != after.modified().ok()
            || !actual.eq_ignore_ascii_case(expected)
        {
            return Err(changed());
        }
        active(cancel)
    }

    fn verify_binding(&self) -> io::Result<()> {
        // The current textual parent is checked only against the already
        // retained parent; it is never used as authority for a child mutation.
        let current = ConfinedDirectory::open_root(&self.parent_path)?;
        if current.identity()? != self.parent.identity()? {
            return Err(changed());
        }
        let reached = self.parent.entry(&self.name)?;
        let class = crate::local_access::classify_open_file(&reached)?;
        if !reached.metadata()?.is_file()
            || class.link_like
            || class.special
            || !super::platform::same_file_object(&reached, &self.file)?
        {
            return Err(changed());
        }
        Ok(())
    }
}

fn changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "published file content, object or parent/name binding changed",
    )
}

fn active(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "publication confirmation canceled",
        ))
    } else {
        Ok(())
    }
}
