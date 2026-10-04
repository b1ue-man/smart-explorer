//! Protected parents are checked for every action, including quick mirror.
use super::omissions::OmissionKind;
use crate::vfs::Backend;
use std::io;

#[derive(Debug)]
struct Boundary(OmissionKind);
impl std::fmt::Display for Boundary {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "protected synchronization entry: {:?}", self.0)
    }
}
impl std::error::Error for Boundary {}
pub(crate) fn omitted(error: &io::Error) -> Option<OmissionKind> {
    error
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<Boundary>())
        .map(|boundary| boundary.0)
        .or_else(|| crate::vfs::omission_reason(error).map(Into::into))
}
pub(crate) fn protected(kind: OmissionKind) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, Boundary(kind))
}
pub(crate) fn guard(
    backend: &dyn Backend,
    root: &str,
    rel: &str,
    cross_mounts: bool,
) -> io::Result<()> {
    if !rel.is_empty() {
        crate::agent_proto::ValidatedRelativePath::parse(rel)?;
    }
    let meta = crate::vfs::sync_stat(backend, root)?;
    if meta.is_symlink {
        return Err(protected(OmissionKind::Link));
    }
    if !meta.is_dir || meta.special {
        return Err(protected(OmissionKind::Special));
    }
    if super::paths::is_engine_name(&meta.name)
        || crate::vfs::is_staging_name(&meta.name)
        || super::snapshot_policy::own_path(backend, root)
    {
        return Err(protected(OmissionKind::OwnFile));
    }
    if rel.is_empty() {
        return Ok(());
    }
    let mut path = root.to_string();
    let parts: Vec<_> = rel.split('/').collect();
    for (index, name) in parts.iter().enumerate() {
        if super::paths::is_engine_name(name) || crate::vfs::is_staging_name(name) {
            return Err(protected(OmissionKind::OwnFile));
        }
        path = crate::vfs::sync_child_path(backend, &path, name)?;
        match crate::vfs::sync_stat(backend, &path) {
            Ok(meta) => {
                if let Some(kind) =
                    super::snapshot_policy::protected(backend, root, &path, &meta, cross_mounts)?
                {
                    return Err(protected(kind));
                }
                if index + 1 < parts.len() && !meta.is_dir {
                    return Err(super::apply_guard::drift(
                        "a synchronization parent changed into a file",
                    ));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
pub(crate) fn target(
    backend: &dyn Backend,
    root: &str,
    rel: &str,
    size: Option<u64>,
) -> io::Result<()> {
    let limits = crate::vfs::target_limits(backend, root);
    if rel.split('/').any(|name| limits.name_issue(name).is_some()) {
        return Err(protected(OmissionKind::NameImpossibleOnTarget));
    }
    if size.is_some_and(|size| !limits.fits_size(size)) {
        return Err(protected(OmissionKind::TooLargeForTarget));
    }
    Ok(())
}

pub(crate) fn deferred(error: &io::Error) -> bool {
    super::apply_guard::is_drift(error)
}

/// Absence is checked under the pair's key policy, including an alternate
/// literal spelling that appeared after the plan was captured.
pub(crate) fn normalized_missing(
    backend: &dyn Backend,
    root: &str,
    rel: &str,
    keys: super::KeyPolicy,
    cross_mounts: bool,
    cancel: &std::sync::atomic::AtomicBool,
) -> io::Result<bool> {
    guard(backend, root, "", cross_mounts)?;
    crate::agent_proto::ValidatedRelativePath::parse(rel)?;
    let mut path = root.to_string();
    let parts: Vec<_> = rel.split('/').collect();
    for (index, wanted) in parts.iter().enumerate() {
        super::transfer_stream::check(cancel)?;
        let parent = crate::vfs::sync_stat(backend, &path)?;
        if !parent.is_dir || parent.is_symlink || parent.special {
            return Err(protected(OmissionKind::Unreadable));
        }
        let listing = crate::vfs::list_dir_tolerant(backend, &path)?;
        let key = keys.key(wanted);
        if listing.omitted.iter().any(|entry| {
            crate::vfs::validate_child_name(&entry.rel).is_err() || keys.key(&entry.rel) == key
        }) {
            return Err(protected(OmissionKind::Unreadable));
        }
        let mut matching = listing
            .entries
            .into_iter()
            .filter(|meta| keys.key(&meta.name) == key);
        let Some(meta) = matching.next() else {
            return Ok(true);
        };
        if matching.next().is_some() {
            return Err(protected(OmissionKind::NameImpossibleOnTarget));
        }
        path = crate::vfs::sync_child_path(backend, &path, &meta.name)?;
        if let Some(kind) =
            super::snapshot_policy::protected(backend, root, &path, &meta, cross_mounts)?
        {
            return Err(protected(kind));
        }
        if index + 1 == parts.len() || !meta.is_dir {
            return Ok(false);
        }
    }
    Ok(false)
}
