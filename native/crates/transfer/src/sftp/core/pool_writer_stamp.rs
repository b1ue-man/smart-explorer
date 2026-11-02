//! The source's modification time on a copy stage, set on its open handle
//! (`FSETSTAT`) right behind the last WRITE and before the FSTAT and CLOSE of
//! the commit. The request goes onto the channel in order with the others
//! (`enqueue`): no round trip of its own, the FSTAT behind it already reports
//! the time as the stage's evidence (stage_evidence.rs), and the server's
//! in-order handling of one handle makes it the last change before the CLOSE.
//!
//! Best effort, as `finish_stage` is for the time: a server that refuses it
//! (not supported, not permitted) keeps the time of the upload, and the engine
//! sees from the evidence that the stage does not carry the time
//! (`engine/publish_time.rs`) and asks `finish_stage` once more.
use super::enqueue;
use super::PoolWriter;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::FileAttributes;
use tokio::task::JoinHandle;

pub(super) type StampTask = JoinHandle<Result<(), SftpError>>;

impl PoolWriter {
    /// Asks the writer to give the stage this time (Unix seconds) at its commit.
    pub(super) fn with_mtime(mut self, seconds: Option<u32>) -> Self {
        self.mtime = seconds;
        self
    }

    /// `FSETSTAT` of the stage's handle onto the channel, if a time was asked.
    pub(super) fn send_stamp(&self) -> Option<StampTask> {
        let seconds = self.mtime?;
        let session = self.channel.session.clone();
        let handle = self.handle.clone();
        let mut attributes = FileAttributes::empty();
        // The v3 ACMODTIME flag writes both fields.
        attributes.atime = Some(seconds);
        attributes.mtime = Some(seconds);
        Some(enqueue(&self.rt, async move {
            session.fsetstat(handle, attributes).await.map(|_| ())
        }))
    }

    /// Waits for the answer; a refusal only leaves the time of the upload.
    pub(super) fn await_stamp(&self, task: StampTask) {
        let _ = self.rt.block_on(task);
    }
}
