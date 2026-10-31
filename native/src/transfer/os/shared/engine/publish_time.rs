//! The source's modification time on a remote stage, before it is published.
//! A copy that kept the time of the copy would read as newer than its own
//! source in the file rule "neuere ersetzen ältere" (`update.rs`), so every
//! stage the engine publishes (upload, remote to remote, server copy, ranges,
//! the stages of remote packets) is given the time of its source first.
//!
//! Providers that take the time only while uploading get it with the stage
//! (`at_upload`); every other target through `vfs::finish_stage`. A target
//! that cannot store a time keeps its own, which is no failure: the rule
//! compares as it did. Packets that publish without a stage on the client
//! (`put_batch` of a Share or agent peer, a single-file packet, a Drive file
//! in a folder this job created) have no place for a time in their wire
//! format and keep the time of the peer.
use super::ops::OpError;
use super::queue::FileWork;
use super::Engine;
use crate::vfs::{Backend, Scheme, StageDurability, StageFinish};

/// Whether the stage writer of `target` takes the source's time itself
/// (WebDAV `X-OC-Mtime`, Drive `modifiedTime`), which saves the request
/// `finish_stage` would send.
pub(super) fn at_upload(target: &dyn Backend, mtime_ms: i64) -> bool {
    mtime_ms > 0 && matches!(target.scheme(), Scheme::Webdav | Scheme::GDrive)
}

/// Gives the complete, closed `stage` the time of `file`'s source. `Ok(true)`:
/// the stage carries it now (what a peer reported about the stage earlier is
/// out of date). The replacement of an existing file is also made durable
/// here; failing that is an error, failing only to store the time is not.
pub(super) fn carry(
    engine: &Engine<'_>,
    target: &dyn Backend,
    file: &FileWork,
    stage: &str,
) -> Result<bool, OpError> {
    if file.mtime_ms <= 0 {
        return Ok(false);
    }
    let durable = file.replacing.is_some() && !engine.resume;
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
