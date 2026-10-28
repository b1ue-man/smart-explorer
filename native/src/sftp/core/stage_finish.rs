//! `finish_stage` of SFTP (MS8, 2026-10-28): the stage's time and mode are
//! set and its data synced on one handle with the requests in flight
//! together. Before, every step waited for the one before it: LSTAT,
//! SETSTAT, LSTAT, then OPEN, `fsync@openssh.com` and CLOSE on a pool
//! channel (6 round trips). Now: LSTAT (the stage must be a regular file
//! before anything opens it: an OPEN of a FIFO would block the server),
//! OPEN, then FSETSTAT, `fsync@openssh.com`, FSTAT, CLOSE and an LSTAT of
//! the path together (3 round trips). The requests reach the channel in
//! that order (enqueue.rs); the server handles one handle's requests in
//! order, so the sync covers the new attributes and FSTAT reads them back.
//!
//! The rules stay: a refused SETSTAT that the server reports as unsupported
//! or not permitted only means the time or mode is not applied; a mode that
//! stays wider than asked is an error; `durable` only after an advertised
//! fsync answered; the path must still be a regular file afterwards
//! (a link swapped in meanwhile is an error). Without a pool channel the
//! path-based steps run as before (`finish_by_path`).
use super::backend::SftpBackend;
use super::channel_pool::note_failure;
use super::enqueue::enqueue;
use super::io_err;
use crate::vfs::{StageDurability, StageFinish, StageFinished, VfsResult};
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, OpenFlags};
use std::io;

pub(super) fn seconds(ms: i64) -> Option<u32> {
    u32::try_from(ms.div_euclid(1_000)).ok()
}

/// A folder, a link or a special file (a type in the mode bits that is
/// none of the known ones).
fn not_regular(attrs: &FileAttributes) -> bool {
    let kind = attrs.file_type();
    kind.is_dir()
        || kind.is_symlink()
        || (kind.is_other() && attrs.permissions.is_some_and(|mode| mode & 0o170000 != 0))
}

/// The attributes `finish` asks for; `atime` keeps the stage's known one
/// (the v3 ACMODTIME flag writes both fields).
fn requested(existing: &FileAttributes, finish: &StageFinish) -> (FileAttributes, Option<u32>) {
    let mtime = finish.mtime_ms.and_then(seconds);
    let mut attributes = FileAttributes::empty();
    if let Some(mtime) = mtime {
        attributes.atime = Some(existing.atime.unwrap_or(mtime));
        attributes.mtime = Some(mtime);
    }
    attributes.permissions = finish.mode.map(|mode| mode & 0o777);
    (attributes, mtime)
}

/// A refused SETSTAT that only means "not applied".
fn tolerated(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Unsupported | io::ErrorKind::PermissionDenied
    )
}

/// The checks on the attributes read back after the finish.
fn judge(
    actual: &FileAttributes,
    finish: &StageFinish,
    mtime: Option<u32>,
    durable: bool,
) -> VfsResult<StageFinished> {
    if let Some(mode) = finish.mode {
        if actual
            .permissions
            .is_none_or(|actual| actual & 0o777 & !(mode & 0o777) != 0)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "SFTP stage could not keep the requested restrictive Unix mode",
            ));
        }
    }
    Ok(StageFinished {
        mtime_applied: mtime.is_some_and(|mtime| actual.mtime == Some(mtime)),
        durable,
    })
}

/// The answer of a request started with `enqueue`.
fn joined<T>(rt: &tokio::runtime::Runtime, task: tokio::task::JoinHandle<T>) -> io::Result<T> {
    rt.block_on(task)
        .map_err(|error| io::Error::other(format!("SFTP-Anfrage abgebrochen: {error}")))
}

fn type_changed() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "SFTP stage type changed during metadata completion",
    )
}

impl SftpBackend {
    pub(super) fn finish_stage_now(
        &self,
        stage: &str,
        finish: StageFinish,
    ) -> VfsResult<StageFinished> {
        let existing = self.lstat_stage(stage)?;
        if not_regular(&existing) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SFTP stage is not a regular file",
            ));
        }
        let (attributes, mtime) = requested(&existing, &finish);
        let set = mtime.is_some() || attributes.permissions.is_some();
        let sync = finish.durability != StageDurability::NotRequired;
        if !set && !sync {
            // Nothing changes: the LSTAT above is the whole answer.
            return Ok(StageFinished {
                mtime_applied: false,
                durable: false,
            });
        }
        let Some(lease) = self.pool.lease(self)? else {
            return self.finish_by_path(stage, &finish, attributes, mtime);
        };
        let channel = lease.channel().clone();
        let opened = self.rt.block_on(channel.session.open(
            stage,
            OpenFlags::WRITE,
            FileAttributes::empty(),
        ));
        let handle = match opened {
            Ok(handle) => handle.handle,
            Err(error) => {
                note_failure(&self.connection, &channel, &error);
                return Err(self.target_error(stage, io_err(error)));
            }
        };
        let session = channel.session.clone();
        let setstat = set.then(|| {
            let (session, handle) = (session.clone(), handle.clone());
            enqueue(&self.rt, async move {
                session.fsetstat(handle, attributes).await
            })
        });
        let fsync = (sync && channel.fsync).then(|| {
            let (session, handle) = (session.clone(), handle.clone());
            enqueue(&self.rt, async move { session.fsync(handle).await })
        });
        let fstat = {
            let (session, handle) = (session.clone(), handle.clone());
            enqueue(&self.rt, async move { session.fstat(handle).await })
        };
        let close = {
            let (session, handle) = (session.clone(), handle);
            enqueue(&self.rt, async move { session.close(handle).await })
        };
        let look = {
            let (session, path) = (session, stage.to_string());
            enqueue(&self.rt, async move { session.lstat(path).await })
        };
        let rt = &self.rt;
        let setstat = setstat.map(|task| joined(rt, task)).transpose()?;
        let fsync = fsync.map(|task| joined(rt, task)).transpose()?;
        let fstat = joined(rt, fstat)?;
        let close = joined(rt, close)?;
        let look = joined(rt, look)?;
        drop(lease);
        let failed = |error: SftpError| {
            note_failure(&self.connection, &channel, &error);
            self.target_error(stage, io_err(error))
        };
        if let Some(Err(error)) = setstat {
            let error = failed(error);
            if !tolerated(&error) {
                return Err(error);
            }
        }
        let durable = match fsync {
            Some(Ok(_)) => true,
            Some(Err(error)) => {
                let error = failed(error);
                if error.kind() != io::ErrorKind::Unsupported {
                    return Err(error);
                }
                false
            }
            None => false,
        };
        let actual = fstat.map_err(&failed)?.attrs;
        close.map_err(&failed)?;
        if not_regular(&look.map_err(&failed)?.attrs) {
            return Err(type_changed());
        }
        judge(&actual, &finish, mtime, durable)
    }

    fn lstat_stage(&self, stage: &str) -> io::Result<FileAttributes> {
        self.safe_sftp_on(|generation| {
            self.rt
                .block_on(generation.sftp().symlink_metadata(stage.to_string()))
        })
        .map(|(_, metadata)| metadata)
    }

    /// The steps one after another on the main session (no pool channel).
    fn finish_by_path(
        &self,
        stage: &str,
        finish: &StageFinish,
        attributes: FileAttributes,
        mtime: Option<u32>,
    ) -> VfsResult<StageFinished> {
        if mtime.is_some() || attributes.permissions.is_some() {
            let generation = self.connection.current()?;
            if let Err(error) = self.rt.block_on(
                generation
                    .sftp()
                    .set_metadata(stage.to_string(), attributes),
            ) {
                self.connection.note_sftp_error(&generation, &error);
                let error = self.target_error(stage, io_err(error));
                if !tolerated(&error) {
                    return Err(error);
                }
            }
        }
        let actual = self.lstat_stage(stage)?;
        if not_regular(&actual) {
            return Err(type_changed());
        }
        // Without a pool channel there is no handle to sync on.
        judge(&actual, finish, mtime, false)
    }
}

#[cfg(test)]
mod tests {
    use super::{judge, requested, seconds, tolerated};
    use crate::vfs::{StageDurability, StageFinish};
    use russh_sftp::protocol::FileAttributes;
    use std::io;

    fn finish(mtime_ms: Option<i64>, mode: Option<u32>) -> StageFinish {
        StageFinish {
            mtime_ms,
            mode,
            durability: StageDurability::NotRequired,
        }
    }

    #[test]
    fn speed_task_sftp_finish_keeps_atime_and_masks_the_mode() {
        let mut existing = FileAttributes::empty();
        existing.atime = Some(7);
        let (attributes, mtime) = requested(&existing, &finish(Some(9_999), Some(0o100640)));
        assert_eq!(mtime, Some(9));
        assert_eq!((attributes.atime, attributes.mtime), (Some(7), Some(9)));
        assert_eq!(attributes.permissions, Some(0o640));
        let (attributes, _) = requested(&FileAttributes::empty(), &finish(Some(5_000), None));
        assert_eq!(attributes.atime, Some(5), "unknown atime: the mtime");
        assert_eq!(seconds(-1), None);
    }

    #[test]
    fn speed_task_sftp_finish_judges_what_the_server_kept() {
        let mut actual = FileAttributes::empty();
        actual.mtime = Some(9);
        actual.permissions = Some(0o100640);
        let done = judge(&actual, &finish(Some(9_000), Some(0o640)), Some(9), true).unwrap();
        assert!(done.mtime_applied && done.durable);
        // A wider mode than asked is an error, never a silent success.
        actual.permissions = Some(0o100644);
        let wider = judge(&actual, &finish(None, Some(0o640)), None, false);
        assert_eq!(wider.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        // Unsupported or refused SETSTAT only means "not applied".
        assert!(tolerated(&io::Error::from(io::ErrorKind::Unsupported)));
        assert!(!tolerated(&io::Error::from(io::ErrorKind::StorageFull)));
    }
}
