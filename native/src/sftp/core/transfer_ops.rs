//! Folder creation and stage cleanup for the transfer engine (plan W1): one
//! MKDIR per folder instead of `mkdir_all`'s walk from the root, a MKDIR
//! that must create the name (the server's `mkdir(2)` refuses an existing
//! one atomically), and removal of a stage this client created with an
//! exclusive OPEN and never published.
use super::backend::SftpBackend;
use super::io_err;
use crate::vfs::{Backend, VfsResult};
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileType, StatusCode};
use std::io;

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
