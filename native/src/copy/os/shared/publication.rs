//! Recovery hooks at the local file publication boundary.
use std::{fs::File, io, path::Path};

/// A hook must persist its evidence before returning success. A failed
/// `before_publish` prevents publication; a failed `after_publish` retains
/// the published destination and any quarantined move source.
pub(crate) trait PublicationObserver {
    /// Persist the exact planned move quarantine before the source name
    /// changes. A conflict may cause a later call with a new private name.
    fn before_source_quarantine(&self, source: &Path, quarantine: &Path) -> io::Result<()> {
        let _ = (source, quarantine);
        Ok(())
    }

    fn before_publish(&self, staged: &Path, target: &Path) -> io::Result<()>;
    fn after_publish(&self, target: &Path) -> io::Result<()>;
    /// The just-published stage/move handle remains live through this hook.
    /// A receipt may use its existing write rights without changing readonly
    /// attributes or assuming that a later path reopen can flush the file.
    fn after_publish_handle(&self, target: &Path, file: &File) -> io::Result<()> {
        let _ = file;
        self.after_publish(target)
    }
    /// A failed preparation may already have durably authorized publication.
    /// Only a durable revocation permits deleting its stage/restoring source.
    fn publication_preparation_failed(&self, staged: &Path, target: &Path) -> io::Result<()> {
        let _ = (staged, target);
        Err(io::Error::new(io::ErrorKind::Unsupported,
            "publication observer cannot confirm durable preparation revocation"))
    }
    /// Called only after live namespace evidence establishes nonpublication.
    /// Persist the negative authorization before stage discard or source restore.
    fn publication_failed(&self, staged: &Path, target: &Path) -> io::Result<()> {
        let _ = (staged, target);
        Err(io::Error::new(io::ErrorKind::Unsupported,
            "publication observer cannot confirm durable publication revocation"))
    }
}

/// Object identity of a regular local file, independent of name, size and
/// timestamps. Unsupported filesystems fail rather than invent an identity.
pub(crate) fn local_file_identity(path: &Path) -> io::Result<String> {
    super::super::platform::stable_path_identity(path)
}

/// The same identity through an already authorized handle. Confined VFS
/// adapters use this boundary without reopening an external path.
pub(crate) fn local_file_identity_handle(file: &File) -> io::Result<String> {
    let metadata = file.metadata()?;
    let class = crate::local_access::classify_open_file(file)?;
    if !metadata.is_file() || class.link_like || class.special {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "file identity requires a regular, non-link file",
        ));
    }
    super::super::platform::stable_handle_identity(file)
}

pub(super) fn before(
    observer: Option<&dyn PublicationObserver>,
    source: &Path,
    target: &Path,
) -> io::Result<()> {
    observer.map_or(Ok(()), |observer| observer.before_publish(source, target))
}

pub(super) fn preparation_failed(
    observer: Option<&dyn PublicationObserver>, staged: &Path, target: &Path,
    cause: &io::Error, quarantine: Option<&super::move_guard::QuarantinedSource>,
) -> io::Result<()> {
    observer.map_or(Ok(()), |observer|
        observer.publication_preparation_failed(staged, target)).map_err(|error| {
        let secured = quarantine.map_or(String::new(), |source|
            format!("; secured move source retained at {}", source.path.display()));
        io::Error::new(error.kind(), format!(
            "publication preparation failed: {cause}; authorization revocation unconfirmed: {error}; publication stage retained at {} for target {}{secured}",
            staged.display(), target.display(),
        ))
    })
}

pub(super) fn after(
    observer: Option<&dyn PublicationObserver>,
    target: &Path,
    file: &File,
    quarantine: Option<&super::move_guard::QuarantinedSource>,
) -> io::Result<()> {
    observer.map_or(Ok(()), |observer| observer.after_publish_handle(target, file)).map_err(|error| {
        let retained = quarantine.map_or(String::new(), |source| {
            format!("; secured move source remains at {}", source.path.display())
        });
        io::Error::new(error.kind(), format!(
            "destination committed at {}{retained}; publication receipt failed: {error}",
            target.display()
        ))
    })
}

pub(super) fn commit_failed(
    observer: Option<&dyn PublicationObserver>, staged: &Path, target: &Path, file: &File,
    cause: &io::Error, quarantine: Option<&super::move_guard::QuarantinedSource>,
) -> io::Result<()> {
    crate::copy::publication_proof::confirm_unpublished(staged, target, file, cause)
        .and_then(|()| observer.map_or(Ok(()), |observer| observer.publication_failed(staged, target)))
        .map_err(|error| {
            let secured = quarantine.map_or(String::new(), |source|
                format!("; secured move source retained at {}", source.path.display()));
            io::Error::new(error.kind(), format!(
                "publication returned {cause}; nonpublication authorization unconfirmed: {error}; cleanup deferred for stage/alias {} and target {}{secured}",
                staged.display(), target.display(),
            ))
        })
}
