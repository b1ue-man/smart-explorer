//! Host side of PutBatch (K18): one stream carries many small new files.
//! Each entry is written into its own private stage, exclusively created and
//! named with the client nonce; only the client's commit (WriteDone)
//! publishes them, never replacing and numbered when a name is taken (K18c).
//! Anything that ends the stream before the commit removes every stage.
//! With `evidence_v1` the host first answers `BatchReady` with the entries'
//! existing destinations and reports each published file's evidence.
use std::io::{self, Write};
use std::time::Duration;

use iroh::endpoint::{RecvStream, SendStream, VarInt};
use tokio::sync::{mpsc, oneshot};

use crate::share::core::eio;
use crate::share::framing::{recv_data_frame, reply, reply_err, TAG_CTRL, TAG_DATA};
use crate::share::fs_access::FsAccess;
use crate::share::host_evidence::Scopes;
use crate::share::io_deadline;
use crate::share::server_transfer::STREAM_BUFFER_CHUNKS;
use crate::share::wire::{Ctrl, FsBatchOutcome, FsBatchPut, FsBatchStatus, FsRequest, FsResponse};

use super::batch_status::{self, BatchKey};
use super::fs_dispatch::BatchAuthority;

#[path = "server_batch_stage.rs"]
mod stage;
use stage::{discard, invalid, open_stage, presence, publish, Parents, Staged};

/// A client finishes its side as soon as it holds the outcomes; one round
/// trip even over a slow relay, with the margin of a control attempt (20 s).
const DELIVERY_WAIT: Duration = Duration::from_secs(20);

/// The stop code of a receive stream dropped without an explicit stop.
const DROPPED_STREAM: VarInt = VarInt::from_u32(0);

pub(super) struct PutBatchJob {
    pub(super) key: BatchKey,
    pub(super) nonce: String,
    pub(super) entries: Vec<FsBatchPut>,
    pub(super) authority: BatchAuthority,
    /// Lease the commit must repeat (mounted streams).
    pub(super) expected_lease: Option<String>,
    /// `evidence_v1`: `BatchReady` and evidence in the outcomes.
    pub(super) evidence: bool,
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
/// the worker has published or discarded every entry; a client that sends
/// nothing for `stall` loses it (the batch is aborted, nothing published).
pub(super) async fn serve<G: Send + 'static>(
    mut send: SendStream,
    mut recv: RecvStream,
    job: PutBatchJob,
    slot: G,
    stall: Duration,
) -> io::Result<()> {
    if let Err(error) = batch_status::begin(&job.key) {
        return reply_err(&mut send, error).await;
    }
    let key = job.key.clone();
    let expected_lease = job.expected_lease.clone();
    let evidence = job.evidence;
    let (commands, receiver) = mpsc::channel(STREAM_BUFFER_CHUNKS);
    let (done_tx, done_rx) = oneshot::channel();
    let (ready_tx, ready_rx) = oneshot::channel();
    let worker = crate::share::blocking::spawn_holding("Share batch upload", slot, move || {
        let _ = done_tx.send(put_worker(job, receiver, ready_tx));
        Ok(())
    });
    // Accepted: the client may already be sending its bytes. An evidence
    // batch names the existing destinations first; nothing is published
    // before the client's commit, which follows only after this reply.
    let ready = if evidence {
        ready_rx
            .await
            .map(|existed| FsResponse::BatchReady { existed })
            .map_err(|_| Stop::Worker)
    } else {
        Ok(FsResponse::Ready)
    };
    let frames = match ready {
        Ok(ready) => match reply(&mut send, ready).await {
            Ok(()) => forward_frames(&mut recv, &commands, expected_lease.as_deref(), stall).await,
            Err(error) => Err(Stop::Transport(error)),
        },
        Err(stop) => Err(stop),
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
            let outcome = reply(&mut send, FsResponse::Batch { status });
            io_deadline::run_for("Share batch outcome", stall, outcome).await?;
            await_delivery(&send, &mut recv, &key).await;
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
    stall: Duration,
) -> Result<(), Stop> {
    loop {
        // Data frames carry at most one chunk; a stalled client releases its
        // admission slot after `stall`, the bound it puts on its own chunks.
        let (tag, payload) = io_deadline::run_for("Share batch data", stall, recv_data_frame(recv))
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

/// Drops the outcome record once the client confirms it received it: it
/// finishes its side, and dropping its stream right after stops ours with
/// code 0 (a client that gives up aborts with its own code instead, and
/// keeps asking for the record). Both arrive together; either one counts.
/// Missing them only kept records until their expiry, and a device whose
/// confirmations kept getting lost this way was refused new batches.
async fn await_delivery(send: &SendStream, recv: &mut RecvStream, key: &BatchKey) {
    let mut probe = [0u8; 1];
    let stopped = send.stopped();
    let confirmed = async {
        tokio::select! {
            biased;
            read = recv.read(&mut probe) => matches!(read, Ok(None)),
            stop = stopped => matches!(stop, Ok(Some(code)) if code == DROPPED_STREAM),
        }
    };
    if let Ok(true) = crate::share::route_protocol::timeout_for(DELIVERY_WAIT, confirmed).await {
        batch_status::delivered(key);
    }
}

fn put_worker(
    job: PutBatchJob,
    mut commands: mpsc::Receiver<PutCommand>,
    ready: oneshot::Sender<Vec<Option<bool>>>,
) -> PutResult {
    if job.evidence {
        let existed = job
            .authority
            .admit(|access| Ok(presence(access, &job.entries)))
            .unwrap_or_else(|_| vec![None; job.entries.len()]);
        let _ = ready.send(existed);
    }
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
    /// Folders resolved while the entries arrive.
    parents: Parents,
    /// Retained when the first stage opens, while the request is alive: it
    /// removes this batch's stages after the client left (`discard`).
    cleanup: Option<FsAccess>,
}

impl<'a> Upload<'a> {
    fn new(job: &'a PutBatchJob) -> Self {
        Self {
            job,
            states: job.entries.iter().map(|_| EntryState::Waiting).collect(),
            current: 0,
            written: 0,
            writer: None,
            parents: Parents::default(),
            cleanup: None,
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
        if self.cleanup.is_none() {
            self.cleanup = job.authority.retained_access();
        }
        let parents = &mut self.parents;
        let opened = job
            .authority
            .admit(|access| open_stage(access, parents, entry, &job.nonce, index));
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
                discard(&staged, self.cleanup.as_ref());
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
                discard(&staged, self.cleanup.as_ref());
            }
            *slot = EntryState::Failed(error);
        }
    }

    /// Abort before the commit: no stage of this batch may remain.
    fn discard_all(&mut self) {
        drop(self.writer.take());
        for state in &self.states {
            if let EntryState::Writing(staged) | EntryState::Staged(staged) = state {
                discard(staged, self.cleanup.as_ref());
            }
        }
    }

    /// Publishes every complete entry in request order, each admitted again;
    /// each folder is resolved again once for the commit.
    fn commit(self) -> Vec<FsBatchOutcome> {
        let job = self.job;
        let mut parents = Parents::default();
        let mut scopes = job.evidence.then(Scopes::default);
        let cleanup = self.cleanup;
        self.states
            .into_iter()
            .map(|state| match state {
                EntryState::Staged(staged) => {
                    let published = job
                        .authority
                        .admit(|access| publish(access, &mut parents, &staged, scopes.as_mut()));
                    match published {
                        Ok((path, evidence)) => FsBatchOutcome::Published { path, evidence },
                        Err(error) => {
                            discard(&staged, cleanup.as_ref());
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
