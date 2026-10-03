use super::backend::SmbBackend;
use super::url::{entry_path, split_path};
use super::{stage_finish, wire};
use crate::vfs::{
    Backend, BackendExtensions, MtimePrecision, NameLimit, StageDurability, StageFinish,
    StageFinished, TargetLimits, VfsListing, VfsResult,
};
use std::io;

impl BackendExtensions for SmbBackend {
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
}
