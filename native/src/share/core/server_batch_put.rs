//! Host side of PutBatch (K18): one stream carries many small new files.
//! Each entry is written into its own private stage, exclusively created and
//! named with the client nonce; only the client's commit (WriteDone)
//! publishes them, never replacing and numbered when a name is taken (K18c).
//! Anything that ends the stream before the commit removes every stage.
use std::io::{self, Write};
use std::time::Duration;

use iroh::endpoint::{RecvStream, SendStream};
use tokio::sync::{mpsc, oneshot};

use crate::share::core::eio;
use crate::share::framing::{recv_tagged, reply, reply_err, TAG_CTRL, TAG_DATA};
use crate::share::fs::ResolvedTarget;
use crate::share::fs_access::FsAccess;
use crate::share::io_deadline;
use crate::share::server_transfer::STREAM_BUFFER_CHUNKS;
use crate::share::wire::{Ctrl, FsBatchOutcome, FsBatchPut, FsBatchStatus, FsRequest, FsResponse};
use crate::vfs::remote_util::{numbered_remote_name, REMOTE_UNIQUE_ATTEMPTS};

use super::batch_status::{self, BatchKey};
use super::fs_dispatch::BatchAuthority;

/// A client finishes its side as soon as it holds the outcomes; one round
/// trip even over a slow relay, with the margin of a control attempt (20 s).
const DELIVERY_WAIT: Duration = Duration::from_secs(20);

pub(super) struct PutBatchJob {
    pub(super) key: BatchKey,
    pub(super) nonce: String,
    pub(super) entries: Vec<FsBatchPut>,
    pub(super) authority: BatchAuthority,
    /// Lease the commit must repeat (mounted streams).
    pub(super) expected_lease: Option<String>,
}

enum PutCommand {
    Data(Vec<u8>),
    Commit,
}

enum PutResult {
    Committed(Vec<FsBatchOutcome>),
    Aborted(io::Error),
}

/// Why the frame loop stopped before a commit reached the worker.
enum Stop {
    /// The stream failed; the client is gone or reset it.
    Transport(io::Error),
    /// The client broke the protocol or its lease.
    Refused(io::Error),
    /// The worker ended first (more bytes than announced).
    Worker,
}

/// The `slot` (one transfer admission for the whole batch) stays held until
/// the worker has published or discarded every entry.
pub(super) async fn serve<G: Send + 'static>(
    mut send: SendStream,
    mut recv: RecvStream,
    job: PutBatchJob,
    slot: G,
) -> io::Result<()> {
    if let Err(error) = batch_status::begin(&job.key) {
        return reply_err(&mut send, error).await;
    }
    let key = job.key.clone();
    let expected_lease = job.expected_lease.clone();
    let (commands, receiver) = mpsc::channel(STREAM_BUFFER_CHUNKS);
    let (done_tx, done_rx) = oneshot::channel();
    let worker = crate::share::blocking::spawn_holding("Share batch upload", slot, move || {
        let _ = done_tx.send(put_worker(job, receiver));
        Ok(())
    });
    // Accepted: the client may already be sending its bytes.
    let frames = match reply(&mut send, FsResponse::Ready).await {
        Ok(()) => forward_frames(&mut recv, &commands, expected_lease.as_deref()).await,
        Err(error) => Err(Stop::Transport(error)),
    };
    drop(commands);
    let result = match done_rx.await {
        Ok(result) => result,
        Err(_) => PutResult::Aborted(eio("Paket-Worker endete ohne Ergebnis")),
    };
    worker.join().await?;
    match (result, frames) {
        (PutResult::Committed(outcomes), _) => {
            let status = FsBatchStatus::Done { outcomes };
            reply(&mut send, FsResponse::Batch { status }).await?;
            await_delivery(&mut recv, &key).await;
            Ok(())
        }
        (PutResult::Aborted(_), Err(Stop::Transport(error))) => Err(error),
        (PutResult::Aborted(_), Err(Stop::Refused(error))) => reply_err(&mut send, error).await,
        (PutResult::Aborted(error), _) => reply_err(&mut send, error).await,
    }
}

/// Forwards data frames to the worker until the client commits.
async fn forward_frames(
    recv: &mut RecvStream,
    commands: &mpsc::Sender<PutCommand>,
    expected_lease: Option<&str>,
) -> Result<(), Stop> {
    loop {
        // A stalled client releases its admission slot after one operation
        // deadline; the client's own per-chunk deadline is the same.
        let (tag, payload) = io_deadline::run("Share batch data", recv_tagged(recv))
            .await
            .map_err(Stop::Transport)?;
        if tag == TAG_DATA {
            commands
                .send(PutCommand::Data(payload))
                .await
                .map_err(|_| Stop::Worker)?;
            continue;
        }
        if tag != TAG_CTRL {
            return Err(Stop::Refused(eio("unerwarteter Frame im Paket")));
        }
        return match serde_json::from_slice::<Ctrl>(&payload) {
            Ok(Ctrl::Fs {
                req: FsRequest::WriteDone,
                lease,
            }) => {
                if expected_lease.is_some() && lease.as_deref() != expected_lease {
                    return Err(Stop::Refused(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "Peer-Mount-Lease fehlt beim Paketabschluss",
                    )));
                }
                commands
                    .send(PutCommand::Commit)
                    .await
                    .map_err(|_| Stop::Worker)
            }
            Ok(_) => Err(Stop::Refused(eio("unerwartete Steuernachricht im Paket"))),
            Err(error) => Err(Stop::Refused(eio(error))),
        };
    }
}

/// Drops the outcome record once the client confirms it received it.
async fn await_delivery(recv: &mut RecvStream, key: &BatchKey) {
    let mut probe = [0u8; 1];
    if let Ok(Ok(None)) = tokio::time::timeout(DELIVERY_WAIT, recv.read(&mut probe)).await {
        batch_status::delivered(key);
    }
}

fn put_worker(job: PutBatchJob, mut commands: mpsc::Receiver<PutCommand>) -> PutResult {
    let mut upload = Upload::new(&job);
    let received = upload.receive(&mut commands);
    // Closing the channel tells the frame loop that nothing more is taken.
    drop(commands);
    let result = match received {
        Ok(()) => PutResult::Committed(upload.commit()),
        Err(error) => {
            upload.discard_all();
            PutResult::Aborted(error)
        }
    };
    let status = match &result {
        PutResult::Committed(outcomes) => FsBatchStatus::Done {
            outcomes: outcomes.clone(),
        },
        PutResult::Aborted(_) => FsBatchStatus::Aborted,
    };
    batch_status::finish(&job.key, status);
    result
}

/// A stage this batch created and still owns.
struct Staged {
    stage: ResolvedTarget,
    /// Peer-namespace folder and name the entry asked for.
    parent: String,
    name: String,
}

enum EntryState {
    Waiting,
    Writing(Staged),
    Staged(Staged),
    Failed(io::Error),
}

struct Upload<'a> {
    job: &'a PutBatchJob,
    states: Vec<EntryState>,
    current: usize,
    written: u64,
    writer: Option<Box<dyn Write + Send>>,
}

impl<'a> Upload<'a> {
    fn new(job: &'a PutBatchJob) -> Self {
        Self {
            job,
            states: job.entries.iter().map(|_| EntryState::Waiting).collect(),
            current: 0,
            written: 0,
            writer: None,
        }
    }

    fn receive(&mut self, commands: &mut mpsc::Receiver<PutCommand>) -> io::Result<()> {
        self.create_empty_entries();
        loop {
            match commands.blocking_recv() {
                Some(PutCommand::Data(payload)) => self.accept(&payload)?,
                Some(PutCommand::Commit) if self.current == self.states.len() => return Ok(()),
                Some(PutCommand::Commit) => return Err(invalid("Paketdaten sind unvollständig")),
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "Paket-Upload wurde abgebrochen",
                    ))
                }
            }
        }
    }

    /// Splits the bytes of one frame across the entries, in request order.
    fn accept(&mut self, mut payload: &[u8]) -> io::Result<()> {
        let job = self.job;
        while !payload.is_empty() {
            let Some(entry) = job.entries.get(self.current) else {
                return Err(invalid("Peer sendet mehr Paketdaten als angekündigt"));
            };
            if self.written == 0
                && matches!(self.states.get(self.current), Some(EntryState::Waiting))
            {
                self.open_current();
            }
            let open = usize::try_from(entry.size - self.written).unwrap_or(usize::MAX);
            let (chunk, rest) = payload.split_at(open.min(payload.len()));
            let failure = self
                .writer
                .as_mut()
                .and_then(|writer| writer.write_all(chunk).err());
            if let Some(error) = failure {
                self.fail_current(error);
            }
            self.written += chunk.len() as u64;
            payload = rest;
            if self.written == entry.size {
                self.close_current();
                self.current += 1;
                self.written = 0;
                self.create_empty_entries();
            }
        }
        Ok(())
    }

    /// Entries without bytes are created as soon as the stream reaches them.
    fn create_empty_entries(&mut self) {
        let job = self.job;
        while job
            .entries
            .get(self.current)
            .is_some_and(|entry| entry.size == 0)
        {
            self.open_current();
            self.close_current();
            self.current += 1;
        }
    }

    fn open_current(&mut self) {
        let job = self.job;
        let index = self.current;
        let Some(entry) = job.entries.get(index) else {
            return;
        };
        let opened = job
            .authority
            .admit(|access| open_stage(access, entry, &job.nonce, index));
        let state = match opened {
            Ok((staged, writer)) => {
                self.writer = Some(writer);
                EntryState::Writing(staged)
            }
            Err(error) => EntryState::Failed(error),
        };
        if let Some(slot) = self.states.get_mut(index) {
            *slot = state;
        }
    }

    /// The entry's bytes are complete: flush and close its stage.
    fn close_current(&mut self) {
        let Some(slot) = self.states.get_mut(self.current) else {
            return;
        };
        if !matches!(slot, EntryState::Writing(_)) {
            return;
        }
        let flushed = match self.writer.take() {
            Some(mut writer) => writer.flush(),
            None => Err(eio("Paketstufe ist nicht geöffnet")),
        };
        let state = std::mem::replace(slot, EntryState::Waiting);
        *slot = match (state, flushed) {
            (EntryState::Writing(staged), Ok(())) => EntryState::Staged(staged),
            (EntryState::Writing(staged), Err(error)) => {
                discard(&staged);
                EntryState::Failed(error)
            }
            (other, _) => other,
        };
    }

    /// Writing failed: the remaining bytes of this entry are skipped.
    fn fail_current(&mut self, error: io::Error) {
        drop(self.writer.take());
        if let Some(slot) = self.states.get_mut(self.current) {
            if let EntryState::Writing(staged) = std::mem::replace(slot, EntryState::Waiting) {
                discard(&staged);
            }
            *slot = EntryState::Failed(error);
        }
    }

    /// Abort before the commit: no stage of this batch may remain.
    fn discard_all(&mut self) {
        drop(self.writer.take());
        for state in &self.states {
            if let EntryState::Writing(staged) | EntryState::Staged(staged) = state {
                discard(staged);
            }
        }
    }

    /// Publishes every complete entry in request order, each admitted again.
    fn commit(self) -> Vec<FsBatchOutcome> {
        let job = self.job;
        self.states
            .into_iter()
            .map(|state| match state {
                EntryState::Staged(staged) => {
                    match job.authority.admit(|access| publish(access, &staged)) {
                        Ok(path) => FsBatchOutcome::Published { path },
                        Err(error) => {
                            discard(&staged);
                            FsBatchOutcome::failed(&error)
                        }
                    }
                }
                EntryState::Failed(error) => FsBatchOutcome::failed(&error),
                EntryState::Waiting | EntryState::Writing(_) => {
                    FsBatchOutcome::failed(&eio("Eintrag wurde nicht vollständig empfangen"))
                }
            })
            .collect()
    }
}

fn open_stage(
    access: &FsAccess,
    entry: &FsBatchPut,
    nonce: &str,
    index: usize,
) -> io::Result<(Staged, Box<dyn Write + Send>)> {
    let (parent, name) = split_entry(&entry.path)?;
    // The client nonce in the name ties the stage to this batch (K18e).
    let stage = access.resolve(&format!("{parent}/{name}.se-batch-{nonce}-{index}"))?;
    let destination = access.resolve(&format!("{parent}/{name}"))?;
    access.require_same_backend(&stage, &destination)?;
    let writer = stage
        .backend
        .open_write_copy_stage_sized(&stage.path, entry.size)?;
    Ok((
        Staged {
            stage,
            parent,
            name,
        },
        writer,
    ))
}

/// Publishes without replacing; a taken name gets the next free number.
fn publish(access: &FsAccess, staged: &Staged) -> io::Result<String> {
    for index in 1..=REMOTE_UNIQUE_ATTEMPTS {
        let path = format!(
            "{}/{}",
            staged.parent,
            numbered_remote_name(&staged.name, index)
        );
        let destination = access.resolve(&path)?;
        access.require_same_backend(&staged.stage, &destination)?;
        match staged
            .stage
            .backend
            .promote_copy_stage(&staged.stage.path, &destination.path)
        {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("Kein freier Name nach {REMOTE_UNIQUE_ATTEMPTS} Versuchen"),
    ))
}

/// Removes an own, never published stage; a failure leaves it in place, as a
/// single upload does.
fn discard(staged: &Staged) {
    let _ = staged.stage.backend.discard_copy_stage(&staged.stage.path);
}

/// Peer-namespace folder and file name of an entry; the file must lie below
/// an exported folder, never be the export itself.
fn split_entry(path: &str) -> io::Result<(String, String)> {
    let mut parts = crate::share::fs_paths::split_clean(path)?;
    let name = parts
        .pop()
        .filter(|_| !parts.is_empty())
        .ok_or_else(|| invalid("Paketeintrag liegt nicht in einem freigegebenen Ordner"))?;
    Ok((format!("/{}", parts.join("/")), name))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.to_string())
}

#[cfg(test)]
mod tests {
    use super::split_entry;

    #[test]
    fn transfer_engine_task_batch_entries_stay_below_an_export() {
        assert_eq!(
            split_entry("/A/dir/datei.txt").unwrap(),
            ("/A/dir".to_string(), "datei.txt".to_string())
        );
        assert_eq!(
            split_entry("//A//x ").unwrap(),
            ("/A".to_string(), "x ".to_string())
        );
        assert!(split_entry("/A").is_err());
        assert!(split_entry("/").is_err());
        assert!(split_entry("/A/../x").is_err());
    }
}
