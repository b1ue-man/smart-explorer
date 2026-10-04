use crate::vfs::{Backend, VfsMeta};
use std::io;

use super::snapshot_hash::md5_hex_to_u64;
use super::types::{Sig, Tree};

#[derive(Clone, Copy, Debug)]
pub(super) enum ExpectedFile {
    Unknown,
    Missing,
    Present(Sig),
}

impl ExpectedFile {
    pub(super) fn from_tree(tree: Option<&Tree>, rel: &str) -> Self {
        match tree {
            None => Self::Unknown,
            Some(tree) => tree.get(rel).copied().map_or(Self::Missing, Self::Present),
        }
    }

    pub(super) fn hash(self) -> u64 {
        match self {
            Self::Present(signature) => signature.hash,
            Self::Unknown | Self::Missing => 0,
        }
    }

    pub(super) fn concretize(
        self,
        backend: &dyn Backend,
        path: &str,
        label: &str,
    ) -> io::Result<Self> {
        if !matches!(self, Self::Unknown) {
            return Ok(self);
        }
        Ok(match current_metadata(backend, path, label)? {
            None => Self::Missing,
            Some(metadata) => Self::Present(Sig {
                size: metadata.size,
                mtime_ms: metadata.mtime_ms,
                hash: metadata
                    .content_md5
                    .as_deref()
                    .map(md5_hex_to_u64)
                    .unwrap_or(0),
            }),
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct CapturedFile {
    pub(super) metadata: Option<VfsMeta>,
}

impl CapturedFile {
    pub(super) fn regular(&self, label: &str) -> io::Result<&VfsMeta> {
        let meta = self.metadata.as_ref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("{label} disappeared"))
        })?;
        if meta.is_symlink {
            return Err(super::apply_boundary::protected(super::OmissionKind::Link));
        }
        if meta.special || meta.is_dir {
            return Err(super::apply_boundary::protected(
                super::OmissionKind::Special,
            ));
        }
        Ok(meta)
    }
}

pub(super) fn capture(
    backend: &dyn Backend,
    path: &str,
    expected: ExpectedFile,
    label: &str,
) -> io::Result<CapturedFile> {
    let metadata = current_metadata(backend, path, label)?;
    match (expected, metadata.as_ref()) {
        (ExpectedFile::Unknown, _) | (ExpectedFile::Missing, None) => {}
        (ExpectedFile::Missing, Some(_)) | (ExpectedFile::Present(_), None) => {
            return Err(drift(&format!("{label} changed since planning")))
        }
        (ExpectedFile::Present(signature), Some(current)) => {
            if current.size != signature.size || current.mtime_ms != signature.mtime_ms {
                return Err(drift(&format!("{label} changed since planning")));
            }
            if let Some(hash) = current.content_md5.as_deref().map(md5_hex_to_u64) {
                if signature.hash != 0 && hash != signature.hash {
                    return Err(drift(&format!("{label} content changed since planning")));
                }
            }
        }
    }
    Ok(CapturedFile { metadata })
}

pub(super) fn revalidate(
    backend: &dyn Backend,
    path: &str,
    captured: &CapturedFile,
    label: &str,
) -> io::Result<()> {
    let current = match captured
        .metadata
        .as_ref()
        .filter(|_| backend.has_duplicate_file_names())
    {
        Some(metadata) => current_identity(backend, path, metadata, label)?,
        None => current_metadata(backend, path, label)?,
    };
    let unchanged = match (captured.metadata.as_ref(), current.as_ref()) {
        (None, None) => true,
        (Some(before), Some(after)) => same_identity(before, after),
        _ => false,
    };
    if unchanged {
        Ok(())
    } else {
        Err(drift(&format!("{label} drifted during apply")))
    }
}

fn regular(metadata: VfsMeta, label: &str) -> io::Result<VfsMeta> {
    if metadata.is_dir || metadata.is_symlink || metadata.special {
        let _ = label;
        return Err(super::apply_boundary::protected(if metadata.is_symlink {
            super::OmissionKind::Link
        } else {
            super::OmissionKind::Special
        }));
    }
    Ok(metadata)
}

fn current_metadata(backend: &dyn Backend, path: &str, label: &str) -> io::Result<Option<VfsMeta>> {
    match crate::vfs::sync_stat(backend, path) {
        Ok(metadata) => {
            let metadata = regular(metadata, label)?;
            if backend.has_duplicate_file_names()
                && metadata.id.as_deref().is_none_or(|id| id.is_empty())
            {
                return Err(drift("ID-addressed file has no stable ID"));
            }
            Ok(Some(metadata))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        // A failed exact-ID/logical-name observation is never repaired with
        // a browsing existence hint that might resolve another identity.
        Err(error) => Err(error),
    }
}

fn current_identity(
    backend: &dyn Backend,
    path: &str,
    previous: &VfsMeta,
    label: &str,
) -> io::Result<Option<VfsMeta>> {
    let id = previous
        .id
        .as_deref()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| drift("ID-addressed file has no stable ID"))?;
    let parent = super::paths::parent_of(path).ok_or_else(|| drift("ID file has no parent"))?;
    let mut matching = super::sync_observation::named(backend, &parent, &previous.name)?
        .into_iter()
        .filter(|meta| meta.id.as_deref() == Some(id));
    let metadata = matching.next().map(|meta| regular(meta, label)).transpose()?;
    if matching.next().is_some() {
        return Err(drift("sync listing repeated a captured identity"));
    }
    Ok(metadata)
}

fn same_identity(before: &VfsMeta, after: &VfsMeta) -> bool {
    before.name == after.name
        && before.size == after.size
        && before.mtime_ms == after.mtime_ms
        && before.id == after.id
        && before.content_md5 == after.content_md5
}

pub(super) fn drift(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, Drift(message.to_string()))
}

#[derive(Debug)]
struct Drift(String);
impl std::fmt::Display for Drift {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str(&self.0)
    }
}
impl std::error::Error for Drift {}

pub(super) fn is_drift(error: &io::Error) -> bool {
    error.get_ref().is_some_and(|inner| inner.is::<Drift>())
}

pub(super) fn current_like(
    backend: &dyn Backend,
    path: &str,
    previous: &CapturedFile,
    label: &str,
) -> io::Result<CapturedFile> {
    if backend.has_duplicate_file_names() {
        if let Some(meta) = &previous.metadata {
            let metadata = current_identity(backend, path, meta, label)?;
            return Ok(CapturedFile { metadata });
        }
    }
    capture(backend, path, ExpectedFile::Unknown, label)
}
