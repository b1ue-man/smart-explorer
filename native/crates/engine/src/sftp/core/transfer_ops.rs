//! Folder creation and stage cleanup for the transfer engine (plan W1): one
//! MKDIR per folder, a MKDIR that must create the name (the server's
//! `mkdir(2)` refuses an existing one atomically), `mkdir_all` from the
//! deepest existing folder (MS7) and removal of a stage this client created
//! with an exclusive OPEN and never published.
//!
//! `mkdir_all` used to send MKDIR and, when refused, LSTAT for every
//! component from the root: two serial round trips per existing folder. It
//! now sends one LSTAT per component at once (one round trip), then MKDIR
//! only for the missing tail. The rule stays: every existing component must
//! be a real folder, never a link (`create_one_dir`'s check), and a missing
//! one is created with that same check against races.
use super::backend::SftpBackend;
use super::io_err;
use crate::vfs::{Backend, VfsResult};
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileType, StatusCode};
use std::io;

/// What one LSTAT of a path component found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Found {
    Folder,
    Missing,
    /// A file, a link or anything else that is no real folder.
    Other,
    /// No answer that decides (permission, transport): ask per component.
    Unknown,
}

/// The components of `path` from the top (`/a`, `/a/b`, `/a/b/c`; relative
/// paths without the leading slash).
pub(super) fn prefixes(path: &str) -> Vec<String> {
    let absolute = path.starts_with('/');
    let mut current = String::new();
    let mut out = Vec::new();
    for part in path.split('/').filter(|part| !part.is_empty()) {
        if !current.is_empty() || absolute {
            current.push('/');
        }
        current.push_str(part);
        out.push(current.clone());
    }
    out
}

/// Where creation starts: the index of the first missing component, after
/// only real folders. `Err(index)`: component `index` is no real folder (a
/// file or a link), which `create_one_dir` reports; `None` when a probe did
/// not decide and the components are asked one by one.
pub(super) fn first_missing(found: &[Found]) -> Option<Result<usize, usize>> {
    for (index, entry) in found.iter().enumerate() {
        match entry {
            Found::Folder => {}
            Found::Missing => return Some(Ok(index)),
            Found::Other => return Some(Err(index)),
            Found::Unknown => return None,
        }
    }
    Some(Ok(found.len()))
}

impl SftpBackend {
    /// The type of the entry at `path` itself (a link is not followed);
    /// `None` when nothing is there. Idempotent, so it runs under the
    /// metadata deadline with one replay after a proven dead transport.
    fn entry_type(&self, path: &str) -> io::Result<Option<FileType>> {
        let path = path.to_string();
        self.connection.safe_metadata(|generation| {
            let path = path.clone();
            Box::pin(async move {
                match generation.sftp().symlink_metadata(path).await {
                    Ok(metadata) => Ok(Some(metadata.file_type())),
                    Err(SftpError::Status(status))
                        if status.status_code == StatusCode::NoSuchFile =>
                    {
                        Ok(None)
                    }
                    Err(error) => Err(error),
                }
            })
        })
    }

    /// One MKDIR. `Ok(Err(refusal))` when the server answered with a status
    /// (the name may exist); a transport failure is the outer error.
    fn make_dir(&self, path: &str) -> io::Result<Result<(), io::Error>> {
        let generation = self.connection.current()?;
        match self
            .rt
            .block_on(generation.sftp().create_dir(path.to_string()))
        {
            Ok(()) => Ok(Ok(())),
            Err(error @ SftpError::Status(_)) => Ok(Err(io_err(error))),
            Err(error) => {
                self.connection.note_sftp_error(&generation, &error);
                Err(io_err(error))
            }
        }
    }

    /// `create_dir` (`exclusive == false`: an existing real folder is fine)
    /// and `create_dir_new` (any existing entry is `AlreadyExists`).
    pub(super) fn create_one_dir(&self, path: &str, exclusive: bool) -> VfsResult<()> {
        let refusal = match self.make_dir(path)? {
            Ok(()) => return Ok(()),
            Err(refusal) => refusal,
        };
        match self.entry_type(path)? {
            // Nothing there: the refusal had another reason (missing parent,
            // no permission) and is reported as the server gave it.
            None => Err(refusal),
            Some(kind) if kind.is_dir() && !exclusive => Ok(()),
            Some(kind) => Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                if kind.is_dir() {
                    format!("{path} existiert bereits")
                } else {
                    format!("{path} existiert bereits und ist kein Ordner")
                },
            )),
        }
    }

    /// `mkdir_all` (MS7): one round trip of LSTATs, then MKDIR for the
    /// missing components only; an undecided probe falls back to the walk
    /// from the top.
    pub(super) fn make_dirs(&self, path: &str) -> VfsResult<()> {
        let parts = prefixes(path);
        let found = self.probe_all(&parts).unwrap_or_default();
        let start = match first_missing(&found).filter(|_| found.len() == parts.len()) {
            Some(Ok(index)) => index,
            // Not a real folder: the same refusal the walk gives.
            Some(Err(index)) => index,
            None => 0,
        };
        for part in &parts[start..] {
            self.create_one_dir(part, false)?;
        }
        Ok(())
    }

    /// LSTAT of every path at once on the main session (requests carry
    /// ids, so they overlap: one round trip).
    fn probe_all(&self, paths: &[String]) -> io::Result<Vec<Found>> {
        let paths = paths.to_vec();
        self.connection.safe_metadata(|generation| {
            let paths = paths.clone();
            Box::pin(async move {
                let sftp = generation.sftp();
                let probes = paths.into_iter().map(|path| sftp.symlink_metadata(path));
                let answers = futures_util::future::join_all(probes).await;
                Ok(answers
                    .into_iter()
                    .map(|answer| match answer {
                        Ok(metadata) if metadata.file_type().is_dir() => Found::Folder,
                        Ok(_) => Found::Other,
                        Err(SftpError::Status(status))
                            if status.status_code == StatusCode::NoSuchFile =>
                        {
                            Found::Missing
                        }
                        Err(_) => Found::Unknown,
                    })
                    .collect())
            })
        })
    }

    /// Removes an unpublished stage. Only a regular file is removed; a stage
    /// that is gone (published or removed) leaves nothing to do.
    pub(super) fn discard_stage(&self, stage: &str) -> VfsResult<()> {
        match self.entry_type(stage)? {
            None => Ok(()),
            Some(kind) if kind.is_file() => self.remove_file(stage),
            Some(_) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Übertragungsstufe {stage} ist keine reguläre Datei und bleibt stehen"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{first_missing, prefixes, Found};

    #[test]
    fn speed_task_sftp_mkdir_all_components_from_the_top() {
        assert_eq!(prefixes("/a/b/c"), ["/a", "/a/b", "/a/b/c"]);
        assert_eq!(prefixes("a//b/"), ["a", "a/b"]);
        assert!(prefixes("/").is_empty());
    }

    #[test]
    fn speed_task_sftp_mkdir_all_starts_at_the_deepest_existing_folder() {
        use Found::{Folder, Missing, Other, Unknown};
        // Everything there: no MKDIR at all.
        assert_eq!(first_missing(&[Folder, Folder, Folder]), Some(Ok(3)));
        // Only the tail is created.
        assert_eq!(first_missing(&[Folder, Missing, Missing]), Some(Ok(1)));
        // A link or a file on the way is refused where the walk refused it.
        assert_eq!(first_missing(&[Other, Folder]), Some(Err(0)));
        assert_eq!(first_missing(&[Folder, Other, Missing]), Some(Err(1)));
        // A probe without a deciding answer: the walk from the top.
        assert_eq!(first_missing(&[Folder, Unknown, Missing]), None);
    }
}
