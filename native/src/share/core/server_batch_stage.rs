//! Stages and publication of one host PutBatch (K18c, K-D). Each distinct
//! folder of a batch is resolved once per phase (accepting the entries, then
//! publishing them) instead of once per stage, destination and attempt; the
//! entries are its children. A child name is a single clean component, its
//! stage is created exclusively and published with a no-replace rename, so a
//! link at either name is never followed: it only takes that name. Copy
//! stages are not forced to stable storage, like the single-file upload
//! (`server_transfer`, `WriteNew`) and the engine's own copies.
use std::collections::HashMap;
use std::io::{self, Write};

use crate::share::fs::ResolvedTarget;
use crate::share::fs_access::FsAccess;
use crate::share::host_evidence::{self, Scopes};
use crate::share::wire::{FsBatchPut, FsFileEvidence};
use crate::vfs::remote_util::{numbered_remote_name, REMOTE_UNIQUE_ATTEMPTS};

/// The folders of one batch phase, each resolved once.
#[derive(Default)]
pub(super) struct Parents(HashMap<String, ResolvedTarget>);

impl Parents {
    /// The entry `name` in the peer folder `parent`: the folder resolved
    /// through `access` (once), the name joined below it.
    pub(super) fn child(
        &mut self,
        access: &FsAccess,
        parent: &str,
        name: &str,
    ) -> io::Result<ResolvedTarget> {
        if crate::share::fs_policy::private_path(&format!("{parent}/{name}")) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Pfad ist nicht freigegeben",
            ));
        }
        let folder = match self.0.get(parent) {
            Some(folder) => folder.clone(),
            None => {
                let folder = access.resolve(parent)?;
                self.0.insert(parent.to_string(), folder.clone());
                folder
            }
        };
        let mut child = folder;
        child.path = format!("{}/{}", child.path.trim_end_matches('/'), name);
        Ok(child)
    }
}

/// A stage this batch created and still owns.
pub(super) struct Staged {
    pub(super) stage: ResolvedTarget,
    /// The stage in the peer namespace, to find it again after the request
    /// ended (`discard`).
    virtual_stage: String,
    /// Peer-namespace folder and name the entry asked for.
    pub(super) parent: String,
    pub(super) name: String,
}

pub(super) fn open_stage(
    access: &FsAccess,
    parents: &mut Parents,
    entry: &FsBatchPut,
    nonce: &str,
    index: usize,
) -> io::Result<(Staged, Box<dyn Write + Send>)> {
    let (parent, name) = split_entry(&entry.path)?;
    // The client nonce in the name ties the stage to this batch (K18e).
    let stage_name = format!("{name}.se-batch-{nonce}-{index}");
    let stage = parents.child(access, &parent, &stage_name)?;
    let virtual_stage = format!("{}/{stage_name}", parent.trim_end_matches('/'));
    let destination = parents.child(access, &parent, &name)?;
    access.require_same_backend(&stage, &destination)?;
    let writer = stage
        .backend
        .open_write_copy_stage_unsynced(&stage.path, entry.size)?;
    Ok((
        Staged {
            stage,
            virtual_stage,
            parent,
            name,
        },
        writer,
    ))
}

/// Whether each entry's requested path exists now (`evidence_v1`); `None`
/// where that cannot be told.
pub(super) fn presence(access: &FsAccess, entries: &[FsBatchPut]) -> Vec<Option<bool>> {
    let mut parents = Parents::default();
    entries
        .iter()
        .map(|entry| {
            let (parent, name) = split_entry(&entry.path).ok()?;
            let target = parents.child(access, &parent, &name).ok()?;
            host_evidence::exists(&target)
        })
        .collect()
}

/// Publishes without replacing; a taken name gets the next free number.
/// With `scopes` the published file's evidence is read back.
pub(super) fn publish(
    access: &FsAccess,
    parents: &mut Parents,
    staged: &Staged,
    scopes: Option<&mut Scopes>,
) -> io::Result<(String, Option<FsFileEvidence>)> {
    for index in 1..=REMOTE_UNIQUE_ATTEMPTS {
        let name = numbered_remote_name(&staged.name, index);
        let path = format!("{}/{}", staged.parent, name);
        let destination = parents.child(access, &staged.parent, &name)?;
        access.require_same_backend(&staged.stage, &destination)?;
        match staged
            .stage
            .backend
            .promote_copy_stage(&staged.stage.path, &destination.path)
        {
            Ok(()) => {
                let evidence = scopes.and_then(|scopes| {
                    host_evidence::of_target(access, &path, &destination, scopes)
                });
                return Ok((path, evidence));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("Kein freier Name nach {REMOTE_UNIQUE_ATTEMPTS} Versuchen"),
    ))
}

/// Removes an own, never published stage. Once the request ended (the
/// client stopped or reset its stream) its live authority refuses every
/// operation, this cleanup included; `cleanup`, the authority retained while
/// the request was alive (sharing, policy generation and rights still
/// checked, only the stream no longer), then removes it. A failure of both
/// leaves the stage in place, as a single upload does.
pub(super) fn discard(staged: &Staged, cleanup: Option<&FsAccess>) {
    if staged
        .stage
        .backend
        .discard_copy_stage(&staged.stage.path)
        .is_ok()
    {
        return;
    }
    if let Some(target) = cleanup.and_then(|access| access.resolve(&staged.virtual_stage).ok()) {
        let _ = target.backend.discard_copy_stage(&target.path);
    }
}

/// Peer-namespace folder and file name of an entry; the file must lie below
/// an exported folder, never be the export itself.
pub(super) fn split_entry(path: &str) -> io::Result<(String, String)> {
    let mut parts = crate::share::fs_paths::split_clean(path)?;
    let name = parts
        .pop()
        .filter(|_| !parts.is_empty())
        .ok_or_else(|| invalid("Paketeintrag liegt nicht in einem freigegebenen Ordner"))?;
    Ok((format!("/{}", parts.join("/")), name))
}

pub(super) fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.to_string())
}

#[cfg(test)]
mod tests {
    use super::split_entry;

    #[test]
    fn transfer_engine_task_batch_entries_stay_below_an_export() {
        assert_eq!(
            split_entry("/A/dir/datei.txt").unwrap(),
            ("/A/dir".to_string(), "datei.txt".to_string())
        );
        assert_eq!(
            split_entry("//A//x ").unwrap(),
            ("/A".to_string(), "x ".to_string())
        );
        assert!(split_entry("/A").is_err());
        assert!(split_entry("/").is_err());
        assert!(split_entry("/A/../x").is_err());
    }
}
