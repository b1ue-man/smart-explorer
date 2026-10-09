//! What happens to the upload stage (`<name>.se-mount-<16 hex>`) of a
//! mounted save that does not reach the user's file name. An application
//! saving through the drive must never find its file under the stage
//! spelling: a save without effect removes its own stage (the spool keeps
//! the content for the retry), and a save that meets a changed remote file
//! becomes a conflict copy that keeps the file name and type.
use super::engine::MountEngine;
use crate::vfs::remote_util::{conflict_rel_name, numbered_remote_name, REMOTE_UNIQUE_ATTEMPTS};
use std::io;

impl MountEngine {
    /// Removes the stage this flush created, when it still is a plain file no
    /// larger than the uploaded spool. Anything else at that spelling is not
    /// provably this flush's stage and stays.
    pub(super) fn discard_own_stage(&self, staged: &str, spool_len: u64) {
        self.invalidate_metadata(staged, false);
        let own = self
            .backend
            .stat(staged)
            .is_ok_and(|meta| !meta.is_dir && !meta.is_symlink && meta.size <= spool_len);
        if own {
            let _ = self.backend.remove_file(staged);
        }
        self.invalidate_metadata(staged, false);
    }

    /// Publishes the uploaded stage as `<name> (Konflikt <time>)[ (n)].<ext>`
    /// next to `remote_path` without replacing anything. `None`: the stage
    /// stays where it is (it holds the only remote copy of this save).
    pub(super) fn publish_conflict_copy(&self, staged: &str, remote_path: &str) -> Option<String> {
        let base = conflict_rel_name(remote_path);
        for index in 1..=REMOTE_UNIQUE_ATTEMPTS {
            let candidate = numbered_path(&base, index);
            match self.backend.promote_staged_no_replace(staged, &candidate) {
                Ok(()) => {
                    self.invalidate_metadata(staged, false);
                    self.invalidate_metadata(&candidate, false);
                    return Some(candidate);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(_) => return None,
            }
        }
        None
    }
}

/// `numbered_remote_name` applied to the last component of `path`.
fn numbered_path(path: &str, index: usize) -> String {
    match path.rsplit_once('/') {
        Some((parent, name)) => format!("{parent}/{}", numbered_remote_name(name, index)),
        None => numbered_remote_name(path, index),
    }
}

#[cfg(test)]
mod tests {
    use super::numbered_path;

    #[test]
    fn mount_save_task_conflict_copies_keep_the_file_type() {
        assert_eq!(
            numbered_path("/Docs/Rechnung (Konflikt 20261009-140305).pdf", 1),
            "/Docs/Rechnung (Konflikt 20261009-140305).pdf"
        );
        assert_eq!(
            numbered_path("/Docs/Rechnung (Konflikt 20261009-140305).pdf", 2),
            "/Docs/Rechnung (Konflikt 20261009-140305) (2).pdf"
        );
        assert_eq!(
            numbered_path("/a.b/Liesmich (Konflikt 1)", 3),
            "/a.b/Liesmich (Konflikt 1) (3)"
        );
        let named = crate::vfs::remote_util::conflict_rel_name("/Docs/Scan.final.pdf");
        assert!(named.starts_with("/Docs/Scan.final (Konflikt "), "{named}");
        assert!(named.ends_with(").pdf"), "{named}");
    }
}
