//! Non-atomic SFTP v3 replacement, separate from the atomic posix-rename
//! callback. The caller journals `retained` before the first request. Every
//! rename uses protocol NoReplace and the original is never unlinked here.
use super::backend::SftpBackend;
use crate::vfs::{Backend, VfsMeta, VfsResult};
use std::io;

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn regular(meta: &VfsMeta) -> bool {
    !meta.is_dir && !meta.is_symlink && !meta.special
}

fn validate(staged: &str, destination: &str, retained: &str) -> VfsResult<()> {
    let name = retained.rsplit('/').next().unwrap_or("");
    let token = name.strip_prefix(".se-replace-").unwrap_or("");
    if staged == destination
        || retained == staged
        || retained == destination
        || parent(staged) != parent(destination)
        || parent(retained) != parent(destination)
        || token.len() != 16
        || !token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || !crate::vfs::is_staging_name(staged.rsplit('/').next().unwrap_or(""))
        || [staged, destination, retained].iter().any(|path| {
            path.contains('\0') || path.split('/').any(|part| matches!(part, "." | ".."))
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "SFTP reversible replacement needs a private stage and .se-replace-16lowerhex sibling",
        ));
    }
    Ok(())
}

fn retained_error(
    error: io::Error,
    staged: &str,
    destination: &str,
    retained: &str,
    detail: &str,
) -> io::Error {
    io::Error::new(error.kind(), format!(
        "{detail}: {error}; no content was deleted; inspect journal paths staged={staged}, destination={destination}, retained={retained}"))
}

impl SftpBackend {
    pub(super) fn replace_retaining_original(
        &self,
        staged: &str,
        destination: &str,
        retained: &str,
    ) -> VfsResult<()> {
        validate(staged, destination, retained)?;
        let stage = self.stat(staged)?;
        let original = self.stat(destination)?;
        if !regular(&stage) || !regular(&original) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SFTP replacement refuses nonregular stages and destinations",
            ));
        }
        if self.try_exists(retained)? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "SFTP retained path is occupied",
            ));
        }
        // An unavailable response may mean this first rename committed. Do
        // not replay or restore from an unconfirmed retained object.
        self.rename_no_replace(destination, retained)
            .map_err(|error| {
                retained_error(
                    error,
                    staged,
                    destination,
                    retained,
                    "Original capture was not confirmed",
                )
            })?;
        let held = self.stat(retained).map_err(|error| {
            retained_error(
                error,
                staged,
                destination,
                retained,
                "Captured original could not be checked",
            )
        })?;
        if !regular(&held) || held.size != original.size || held.mtime_ms != original.mtime_ms {
            return Err(retained_error(
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Captured metadata differs from the original snapshot",
                ),
                staged,
                destination,
                retained,
                "Keep the captured object for journal recovery",
            ));
        }
        match self.rename_no_replace(staged, destination) {
            Ok(()) => Ok(()),
            Err(publish) => {
                // NoReplace restores only into a still-free destination. If
                // the publish committed or a third party filled the slot,
                // restoration fails safely and both contents stay known.
                let restoration = self.rename_no_replace(retained, destination);
                let detail = match restoration {
                    Ok(()) => "Stage publication failed; original restored without replacing".to_string(),
                    Err(error) => format!("Stage publication failed; restore was refused or ambiguous ({error}); inspect the known retained and destination paths"),
                };
                Err(retained_error(
                    publish,
                    staged,
                    destination,
                    retained,
                    &detail,
                ))
            }
        }
    }
}
