use super::backend::SmbBackend;
use super::url::{entry_path, split_path};
use super::{stage_finish, wire};
use crate::vfs::{
    Backend, BackendExtensions, MtimePrecision, NameLimit, RecursiveDeleteFailure,
    RecursiveDeleteProgress, RecursiveDeleteReport, StageDurability, StageFinish, StageFinished,
    TargetLimits, VfsListing, VfsResult,
};
use std::io;
use std::sync::atomic::AtomicBool;

impl BackendExtensions for SmbBackend {
    fn directory_reservation_supported(&self, root: &str) -> bool {
        // New directories are entries inside a share, never new SMB shares.
        split_path(root).is_ok_and(|path| path.is_some())
    }

    fn create_dir_exclusive(&self, path: &str) -> VfsResult<()> {
        super::folder_reservation::create(self, path)
    }

    fn directory_identity(&self, path: &str) -> VfsResult<Option<String>> {
        super::folder_reservation::identity(self, path)
    }

    fn capacity(&self, root: &str) -> VfsResult<crate::vfs::CapacitySnapshot> {
        super::capacity::query(self, root)
    }
    fn target_limits(&self, _root: &str) -> TargetLimits {
        // SMB's FILETIME wire precision is 100 ns, but the server may host
        // FAT or another coarse filesystem. No volume type was negotiated.
        TargetLimits {
            windows_names: true,
            max_name: Some(NameLimit::Utf16Units(255)),
            mtime_precision: MtimePrecision::Unknown,
            max_file_size: None,
        }
    }

    fn list_dir_tolerant(&self, path: &str) -> VfsResult<VfsListing> {
        let Some(target) = split_path(path)? else {
            return self.list_dir(path).map(VfsListing::complete);
        };
        let rel = target.rel;
        self.session
            .read(&target.share, "Ordner lesen", path, |conn, tree| {
                let rel = rel.clone();
                async move { wire::list_tolerant(&conn, &tree, &rel).await }
            })
    }

    fn finish_stage(&self, stage: &str, finish: StageFinish) -> VfsResult<StageFinished> {
        let metadata = self.stat(stage)?;
        if metadata.is_dir || metadata.is_symlink || metadata.special {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SMB stage must be a regular file",
            ));
        }
        let target = entry_path(stage)?;
        let mut applied = false;
        if let Some(ms) = finish.mtime_ms {
            if let Some(ticks) = stage_finish::filetime(ms) {
                let rel = target.rel.clone();
                let result =
                    self.session.write(
                        &target.share,
                        "Änderungszeit setzen",
                        stage,
                        |conn, tree| async move {
                            stage_finish::set_mtime(&conn, &tree, &rel, ticks).await
                        },
                    );
                match result {
                    Ok(()) => {
                        let actual = self.stat(stage)?;
                        if actual.is_dir || actual.is_symlink || actual.special {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "SMB stage type changed during metadata completion",
                            ));
                        }
                        applied = actual.mtime_ms == ms;
                    }
                    // Some SMB servers/accounts do not allow timestamp
                    // changes; only this feature falls back to baseline.
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::Unsupported
                                | io::ErrorKind::PermissionDenied
                                | io::ErrorKind::InvalidInput
                        ) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        let durable = if finish.durability == StageDurability::NotRequired {
            false
        } else {
            let rel = target.rel;
            match self.session.write(
                &target.share,
                "Datei dauerhaft schreiben",
                stage,
                |conn, tree| async move { stage_finish::flush(&conn, &tree, &rel).await },
            ) {
                Ok(()) => true,
                Err(error) if error.kind() == io::ErrorKind::Unsupported => false,
                Err(error) => return Err(error),
            }
        };
        Ok(StageFinished {
            mtime_applied: applied,
            durable,
        })
    }

    /// A folder inside a share: one listing per folder and concurrent
    /// deletes (tree_remove.rs). The share root and the server level are no
    /// entry (`entry_path`) and keep the generic walk unchanged.
    fn supports_remove_tree(&self, path: &str) -> VfsResult<bool> {
        Ok(entry_path(path).is_ok())
    }

    fn remove_tree(
        &self,
        path: &str,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(RecursiveDeleteProgress),
    ) -> Result<RecursiveDeleteReport, RecursiveDeleteFailure> {
        let share = match entry_path(path) {
            Ok(target) => target.share,
            Err(error) => {
                return Err(RecursiveDeleteFailure {
                    error,
                    planned: 0,
                    removed: 0,
                })
            }
        };
        crate::vfs::remove_tree_listed(self, path, cancel, progress, |leaves| {
            self.remove_tree_leaves(&share, leaves)
        })
    }

    /// The stage's evidence its writer recorded at the commit
    /// (stage_evidence.rs, G3-02).
    fn take_stage_evidence(&self, stage: &str) -> Option<crate::vfs::StageEvidence> {
        self.written.take(stage)
    }

    /// One compound publishes the stage and describes the published file;
    /// a stage without evidence is published as before.
    fn promote_copy_stage_evidence(
        &self,
        staged: &str,
        destination: &str,
    ) -> VfsResult<Option<crate::vfs::FileEvidence>> {
        let Some(proven) = self.written.proven(staged) else {
            return self.promote_copy_stage(staged, destination).map(|()| None);
        };
        let (from, to) = super::url::rename_paths(staged, destination)?;
        let (from_rel, to_rel) = (from.rel, to.rel);
        let published =
            self.session
                .write(&from.share, "Umbenennen", staged, |conn, tree| async move {
                    super::stage_evidence::rename_no_replace_evidenced(
                        &conn, &tree, &from_rel, &to_rel,
                    )
                    .await
                })?;
        self.written.forget(staged);
        Ok(
            super::stage_evidence::regular_evidence(destination, &published)
                .filter(|evidence| evidence.size == proven.size),
        )
    }
}
