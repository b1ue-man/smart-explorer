//! `content_md5_many` of a plain SFTP server: the SFTP attributes of every
//! file before (pipelined LSTATs), one exec request per group of files
//! (digest_batch.rs), the attributes again after (pipelined), and a digest
//! only for a file that is still the listed one: a regular file of the
//! listed size and time before and after, of which the shell read that size.
//! Every other file gets `None` and the caller takes its own way for it
//! (`content_md5` of digest_command.rs, then the content), so its error or
//! omission is the one it always was.
//!
//! The gates of the connection stay in charge: the MD5 gate
//! (`SftpBackend::md5_gate`) closes for everything that proves that this
//! connection cannot hash (exec refused, no marker, no tool, the shell sees
//! another file under a path), and then neither this way nor the single-file
//! way asks again. A group answer that does not line up only counts against
//! the group gate (`md5_batch_gate`); after three in a row the connection
//! asks file by file as before.
use super::backend::SftpBackend;
use super::digest_batch::{batch_command_line, groups, interpret_batch, Answer, BatchReply};
use super::digest_command::{wait_for, DigestGate};
use super::exec::CapturedExec;
use super::metadata::{basename, to_vfs};
use crate::vfs::{VfsMeta, VfsResult};
use futures_util::future::join_all;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// LSTATs in flight together on the main session.
const STATS_IN_FLIGHT: usize = 256;

/// The two gates of one connection.
pub(super) struct Gates<'a> {
    pub(super) md5: &'a DigestGate,
    pub(super) batch: &'a DigestGate,
}

/// The listed file the shell's answer is about, for the checks after it.
enum Pending {
    Digest(String, u64),
    NotRegular,
}

/// The whole call with the world behind two closures: `stat_many` (the SFTP
/// attributes of paths, `None` where there are none) and `run` (the fixed
/// command line, its standard input and the wait; the exec result). `files`
/// pairs each path with the metadata its listing showed.
pub(super) fn md5_many_by(
    gates: Gates<'_>,
    files: &[(&str, &VfsMeta)],
    cancel: &AtomicBool,
    mut stat_many: impl FnMut(&[&str]) -> Vec<Option<VfsMeta>>,
    mut run: impl FnMut(&str, &[u8], Duration) -> io::Result<Option<CapturedExec>>,
) -> VfsResult<Vec<Option<String>>> {
    let mut out = vec![None; files.len()];
    if files.is_empty() || gates.md5.closed() || gates.batch.closed() {
        return Ok(out);
    }
    let paths: Vec<&str> = files.iter().map(|(path, _)| *path).collect();
    let before = stat_many(&paths);
    let eligible: Vec<(usize, &str, u64)> = files
        .iter()
        .zip(&before)
        .enumerate()
        .filter(|(_, ((_, listed), now))| now.as_ref().is_some_and(|now| same(listed, now)))
        .map(|(index, ((path, listed), _))| (index, *path, listed.size))
        .collect();
    let command = batch_command_line();
    let mut pending: Vec<(usize, Pending)> = Vec::new();
    for group in groups(&eligible) {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "content digest canceled",
            ));
        }
        if gates.md5.closed() || gates.batch.closed() {
            break;
        }
        let captured = match run(&command, &group.stdin, wait_for(group.bytes)) {
            Ok(Some(captured)) => captured,
            // The server gives no channel while no other digest command
            // runs here (exec_slots.rs): these files go their own way.
            Ok(None) => continue,
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                gates.md5.give_up();
                return Ok(vec![None; files.len()]);
            }
            Err(error) if error.kind() == io::ErrorKind::TimedOut => {
                gates.md5.failed();
                continue;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            // A transport failure: the single-file way reports it per file.
            Err(_) => break,
        };
        match interpret_batch(&captured, group.members.len()) {
            BatchReply::Unavailable => {
                gates.md5.give_up();
                return Ok(vec![None; files.len()]);
            }
            BatchReply::Failed(_) => gates.batch.failed(),
            BatchReply::Answers { answers, aligned } => {
                if aligned {
                    gates.batch.worked();
                } else {
                    gates.batch.failed();
                }
                for (index, answer) in group.members.iter().zip(answers) {
                    match answer {
                        Answer::Digest { hex, size } => {
                            pending.push((*index, Pending::Digest(hex, size)))
                        }
                        Answer::NotRegular => pending.push((*index, Pending::NotRegular)),
                        Answer::Missing => {}
                    }
                }
            }
        }
    }
    if pending.is_empty() {
        return Ok(out);
    }
    let asked: Vec<&str> = pending.iter().map(|(index, _)| paths[*index]).collect();
    let after = stat_many(&asked);
    for ((index, answer), now) in pending.into_iter().zip(after) {
        let listed = files[index].1;
        // Changed while it was hashed: the caller's own way decides.
        let Some(now) = now.filter(|now| same(listed, now)) else {
            continue;
        };
        match answer {
            Pending::Digest(hex, size) if size == now.size => out[index] = Some(hex),
            // The same SFTP attributes, another file for the shell (another
            // root or chroot): this connection's shell view is not SFTP's.
            Pending::Digest(..) | Pending::NotRegular => {
                gates.md5.give_up();
                return Ok(vec![None; files.len()]);
            }
        }
    }
    if out.iter().any(Option::is_some) {
        gates.md5.worked();
    }
    Ok(out)
}

/// The SFTP attributes show the listed regular file: kind, size and time.
fn same(listed: &VfsMeta, now: &VfsMeta) -> bool {
    !now.is_dir
        && !now.is_symlink
        && !now.special
        && now.size == listed.size
        && now.mtime_ms == listed.mtime_ms
        && now.id == listed.id
}

impl SftpBackend {
    /// `BackendExtensions::content_md5_many` for this connection.
    pub(super) fn md5_many_by_command(
        &self,
        files: &[(&str, &VfsMeta)],
        cancel: &AtomicBool,
    ) -> VfsResult<Vec<Option<String>>> {
        md5_many_by(
            Gates {
                md5: &self.md5_gate,
                batch: &self.md5_batch_gate,
            },
            files,
            cancel,
            |paths| self.stat_many(paths),
            |command, stdin, wait| super::digest_exec::run(self, command, stdin, wait, cancel),
        )
    }

    /// LSTAT of every path, many in flight on the main session (russh-sftp
    /// matches every answer to its request id, tree_remove.rs); `None` where
    /// the server answered with an error or the connection failed.
    fn stat_many(&self, paths: &[&str]) -> Vec<Option<VfsMeta>> {
        let Ok(generation) = self.connection.current() else {
            return paths.iter().map(|_| None).collect();
        };
        let (connection, generation) = (&self.connection, &generation);
        let mut out = Vec::with_capacity(paths.len());
        for chunk in paths.chunks(STATS_IN_FLIGHT) {
            let found = self
                .rt
                .block_on(join_all(chunk.iter().map(|path| async move {
                    generation
                        .sftp()
                        .symlink_metadata(path.to_string())
                        .await
                        .map_err(|error| connection.note_sftp_error(generation, &error))
                })));
            out.extend(
                chunk
                    .iter()
                    .zip(found)
                    .map(|(path, found)| found.ok().map(|meta| to_vfs(basename(path), &meta))),
            );
        }
        out
    }
}
