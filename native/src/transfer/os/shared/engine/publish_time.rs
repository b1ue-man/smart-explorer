//! The source's modification time on a remote stage, before it is published.
//! A copy that kept the time of the copy would read as newer than its own
//! source in the file rule "neuere ersetzen ältere" (`update.rs`), so every
//! stage the engine publishes (upload, remote to remote, server copy, ranges,
//! the stages of remote packets) is given the time of its source first, on
//! every copy, whether it follows the rule or not (a merge back would
//! otherwise replace every copy; Windows `CopyFile2` always did).
//!
//! The time costs no request of its own wherever the protocol carries it in a
//! step that is sent anyway:
//! - WebDAV (`X-OC-Mtime`) and Drive (`modifiedTime`) take it with the stage
//!   upload (`at_upload`);
//! - SFTP sets it on the open handle right behind the last WRITE, before the
//!   CLOSE (`FSETSTAT`, pipelined: no round trip of its own); the `FSTAT` that
//!   follows in the same pipeline reports it, and that evidence shows the
//!   engine that the stage carries the time (`written_with_time`), so the
//!   stage's evidence stays valid; an SMB stage that fits one WRITE puts a
//!   SET_INFO into its commit compound the same way;
//! - a Share host with `stage_mtime_v1` takes it with the stage's `WriteNew`
//!   and sets it at the commit before it reads the stage's evidence; an SSH
//!   agent with `stage-mtime-v1` takes it with `WriteNewTimed`, sets it on the
//!   open stage and answers the commit with the stage's evidence. Either
//!   evidence shows the time, so no `FinishStage` follows and the evidence
//!   stays the stage's receipt;
//! - packets that publish without a stage on the client (`put_batch` of
//!   Share, agent, SFTP and SMB peers, a single-file packet) name each file's
//!   time in the packet (`BatchPut::mtime_ms`); the peer sets it before
//!   publishing where it reports `vfs::batch_put_times`.
//!
//! Where the protocol has no such place the stage gets it with `finish_stage`
//! after the write (`carry`): a limit documented in `docs/TRANSFER_ENGINE.md`
//! (stages of older Share hosts and agents and of the background service,
//! ranged stages, FTP `MFMT`, SMB stages larger than one WRITE, SFTP without
//! a pool channel). A target
//! that cannot store a time keeps its own, which is no failure: the rule
//! compares as it did. A Drive file in a folder this job created (written
//! without a stage) has no place for a time and keeps the provider's.
use super::ops::OpError;
use super::queue::FileWork;
use super::Engine;
use crate::vfs::{Backend, Scheme, StageDurability, StageEvidence, StageFinish};

/// Whether the stage writer of `target` takes the source's time itself with
/// the upload, so `open_write_copy_stage_timed` is used and no request of
/// its own follows: WebDAV and Drive always; SFTP (and SSH agents), SMB and
/// Share hosts for stages that are not replacements (a replacement is made
/// durable by `finish_stage` anyway). A Share host or agent without the
/// capability gets the plain sized stage from that call.
pub(super) fn at_upload(target: &dyn Backend, mtime_ms: i64, durable: bool) -> bool {
    mtime_ms > 0
        && match target.scheme() {
            Scheme::Webdav | Scheme::GDrive => true,
            Scheme::Sftp | Scheme::Smb | Scheme::Peer => !durable,
            _ => false,
        }
}

/// Whether the writer of the stage stored the source's time already: the
/// providers that take it only with the upload, or a stage whose reported
/// evidence (SFTP `FSTAT` behind the `FSETSTAT`, the SMB CLOSE's attributes,
/// the Share host's `Written`, the agent's `StageWritten`) shows it: to the second where the protocol stores seconds (SFTP), else to
/// the millisecond, so a stage written at nearly the source's time is not
/// mistaken for one that carries it.
fn written_with_time(target: &dyn Backend, mtime_ms: i64, known: Option<&StageEvidence>) -> bool {
    match target.scheme() {
        Scheme::Webdav | Scheme::GDrive => true,
        scheme => known.is_some_and(|known| {
            let tolerance = if scheme == Scheme::Sftp { 999 } else { 0 };
            (i128::from(known.stage.mtime_ms) - i128::from(mtime_ms)).abs() <= tolerance
        }),
    }
}

/// Gives the complete, closed `stage` the time of `file`'s source unless its
/// writer did (`known`: the evidence the peer reported for the stage).
/// `Ok(true)`: `finish_stage` stored it, so what a peer reported about the
/// stage earlier is out of date. The replacement of an existing file is also
/// made durable here; failing that is an error, failing only to store the
/// time is not.
pub(super) fn carry(
    engine: &Engine<'_>,
    target: &dyn Backend,
    file: &FileWork,
    stage: &str,
    known: Option<&StageEvidence>,
) -> Result<bool, OpError> {
    if file.mtime_ms <= 0 {
        return Ok(false);
    }
    let durable = file.replacing.is_some() && !engine.resume;
    if !durable && written_with_time(target, file.mtime_ms, known) {
        return Ok(false);
    }
    let finish = StageFinish {
        mtime_ms: Some(file.mtime_ms),
        mode: None,
        durability: if durable {
            StageDurability::Now
        } else {
            StageDurability::NotRequired
        },
    };
    match crate::vfs::finish_stage(target, stage, finish) {
        Ok(finished) => Ok(finished.mtime_applied),
        Err(error) if durable => Err(OpError::target(error)),
        Err(error) => {
            engine.issues.note(
                &file.source,
                &format!("Änderungszeit der Quelle nicht übernommen: {error}"),
            );
            Ok(false)
        }
    }
}
