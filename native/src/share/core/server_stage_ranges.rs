//! Ranged uploads on the Share host (`ranges_v1`, E13-S2): the three requests
//! `SegmentedStageOpen`, `StageWriteAt` and `SegmentedStageFinish`, served
//! from the stage table of the connection (`RangeStages`).
//!
//! * The host alone picks the stage name (`<path>.se-upload-<16 hex from the
//!   system's random source>`) and creates the file exclusively with the
//!   checks of `WriteNew`: the export and the live access right
//!   (`resolve_write`), the policy of the target, the room's read-only rule
//!   (the dispatcher refuses all three requests there) and the free space,
//!   which the table adds up over every open stage.
//! * The handle of that creation stays in the table. Every range is written
//!   through it; the stage is never opened again by path.
//! * A range is entered as taken when it is accepted; an overlap with a taken
//!   or finished range is refused.
//! * The write right is checked again at every range, at every chunk and at
//!   the finish; when it is gone, the request is refused and the stage removed.
//! * The stage belongs to the connection that opened it (and so to its
//!   principal); at most `admission` stages are open per connection (`Busy`
//!   beyond), and a stage goes with its connection, after a stall, and at
//!   the next start (`Journal`).
use std::io;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::agent_proto::{random_stage_name, Claim, Journal, RangeStages};
use crate::share::blocking;
use crate::share::framing::lz4_stream::{self, classify, Content};
use crate::share::framing::{recv_content_frame_or_end, reply, reply_err};
use crate::share::fs_access::FsAccess;
use crate::share::fs_path_adapter;
use crate::share::io_deadline;
use crate::share::mount_lease::{run_authorized, MountLeaseAuthorization};
use crate::share::server_transfer::STREAM_BUFFER_CHUNKS;
use crate::share::session::PeerPrincipal;
use crate::share::wire::{FsRequest, FsResponse, FsSegmentedFinish, FsSegmentedOpen, FsStageWrite};

use super::control_operations::control;
use super::FsStream;

/// A stage that makes no progress for this long is removed. A sender can
/// wait for its own flow permits between two ranges, so this is longer than
/// the stall of one chunk.
pub(in crate::share) const STAGE_STALL: std::time::Duration = std::time::Duration::from_secs(300);
/// Names that are free are looked for this often (64 random bits each).
const NAME_ATTEMPTS: usize = 4;

/// How often a connection looks for stages without progress.
const SWEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(30);

/// This process's list of unfinished stages, kept in the app's data folder;
/// none when the folder cannot be used.
fn journal() -> Option<Arc<Journal>> {
    static JOURNAL: std::sync::OnceLock<Option<Arc<Journal>>> = std::sync::OnceLock::new();
    JOURNAL
        .get_or_init(|| Journal::open(crate::support_dirs::app_data_dir().join("range-stages")))
        .clone()
}

/// The stage table of one connection: it lives as long as the connection
/// (the guard is held by the accept loop), looks for stalled stages while it
/// lives and removes every unfinished stage when it ends.
pub(in crate::share) struct ConnectionStages {
    stages: Arc<RangeStages>,
    sweeper: tokio::task::JoinHandle<()>,
}

impl ConnectionStages {
    /// Must be called inside the runtime of the node.
    pub(in crate::share) fn start() -> Self {
        let stages = RangeStages::new(
            crate::share::keepalive::TRANSFER_STREAMS_PER_CONNECTION as usize,
            STAGE_STALL,
            Some(journal),
        );
        let table = stages.clone();
        let sweeper = tokio::spawn(async move {
            let mut tick = tokio::time::interval(SWEEP_EVERY);
            loop {
                tick.tick().await;
                let table = table.clone();
                if tokio::task::spawn_blocking(move || table.sweep())
                    .await
                    .is_err()
                {
                    return;
                }
            }
        });
        Self { stages, sweeper }
    }

    pub(in crate::share) fn table(&self) -> Arc<RangeStages> {
        self.stages.clone()
    }
}

impl Drop for ConnectionStages {
    fn drop(&mut self) {
        self.sweeper.abort();
        self.stages.close();
    }
}

/// A full table is congestion (the client backs off); everything else is
/// reported as it is.
fn wire(error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::ResourceBusy {
        return crate::vfs::congestion_error(error.to_string(), None);
    }
    error
}

fn refused(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.to_string())
}

/// Serves one of the three requests of ranged uploads.
pub(super) async fn serve(stream: FsStream, request: FsRequest) -> io::Result<()> {
    match request {
        FsRequest::SegmentedStageOpen(request) => open(stream, request).await,
        FsRequest::StageWriteAt(request) => write_at(stream, request).await,
        FsRequest::SegmentedStageFinish(request) => finish(stream, request).await,
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Keine Anfrage für Bereichsstufen",
        )),
    }
}

/// `SegmentedStageOpen`: creates the stage and answers with its path.
pub(super) async fn open(stream: FsStream, request: FsSegmentedOpen) -> io::Result<()> {
    let FsStream {
        mut send,
        access,
        authorization,
        context,
        principal,
        ..
    } = stream;
    let stages = context.stages.clone();
    let result = control(&principal, "Share open range stage", move || {
        run_authorized(authorization.as_ref(), || {
            create_stage(&access, &stages, &request)
        })
    })
    .await;
    match result {
        Ok(stage) => reply(&mut send, FsResponse::SegmentedStageReady { stage }).await,
        Err(error) => reply_err(&mut send, wire(error)).await,
    }
}

fn create_stage(
    access: &FsAccess,
    stages: &Arc<RangeStages>,
    request: &FsSegmentedOpen,
) -> io::Result<String> {
    let parts = crate::share::fs_paths::split_clean(&request.path)?;
    if parts.len() < 2 {
        return Err(refused("Pfad ist kein Dateiname"));
    }
    for _ in 0..NAME_ATTEMPTS {
        let stage = random_stage_name(&request.path)?;
        let target = access.resolve_write(&stage)?;
        if !target.backend.is_local() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Diese Freigabe kann keine Bereiche schreiben",
            ));
        }
        // Also the read guards of the provider; a taken name is not used.
        if target.backend.try_exists(&target.path)? {
            continue;
        }
        let path = fs_path_adapter::to_os_path(&target.path);
        let parent = path
            .parent()
            .ok_or_else(|| refused("Die Stufe hat keinen Ordner"))?;
        let pending = stages.begin(parent, request.size)?;
        let file = match fs_path_adapter::create_stage_file(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        if let Err(error) = file.set_len(request.size) {
            // The stage was created here and its handle is still held: it
            // is entered, so the table removes it again (identity-checked).
            pending.commit(stage.clone(), path, file)?;
            stages.discard(&stage);
            return Err(error);
        }
        pending.commit(stage.clone(), path, file)?;
        return Ok(stage);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Keine freie Upload-Stufe",
    ))
}

/// One command of a range's writer.
enum Command {
    Data(Vec<u8>),
    Complete,
}

/// `StageWriteAt`: accepts the range, says `Ready`, receives exactly `len`
/// bytes (plain or as LZ4 blocks), writes them through the stage's handle and
/// answers `Ok` once the sender ended its stream.
pub(super) async fn write_at(stream: FsStream, request: FsStageWrite) -> io::Result<()> {
    let FsStream {
        mut send,
        mut recv,
        access,
        authorization,
        context,
        principal,
    } = stream;
    if request.len == 0 || request.stage.is_empty() {
        return reply_err(&mut send, refused("Leerer Bereich")).await;
    }
    let slot = match context.admit(&send).await {
        Ok(slot) => slot,
        Err(error) => return reply_err(&mut send, error).await,
    };
    let stages = context.stages.clone();
    let claim = {
        let (stages, access) = (stages.clone(), access.clone());
        let (stage, offset, len) = (request.stage.clone(), request.offset, request.len);
        control(&principal, "Share accept range", move || {
            run_authorized(authorization.as_ref(), || {
                admit_range(&access, &stages, &stage, offset, len)
            })
        })
        .await
    };
    let claim = match claim {
        Ok(claim) => claim,
        Err(error) => return reply_err(&mut send, error).await,
    };
    reply(&mut send, FsResponse::Ready).await?;

    let lz4 = lz4_stream::negotiated(request.lz4);
    let stall = context.stall();
    let (tx, rx) = mpsc::channel(STREAM_BUFFER_CHUNKS);
    let worker_access = access.clone();
    let worker = blocking::spawn_holding("Share stage range write", slot, move || {
        write_range(claim, worker_access, rx)
    });
    let received = receive_range(&mut recv, &tx, request.len, lz4, stall).await;
    let outcome = match received {
        Ok(()) => {
            if tx.send(Command::Complete).await.is_err() {
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "Der Bereichsschreiber ist beendet",
                ))
            } else {
                Ok(())
            }
        }
        Err(error) => Err(error),
    };
    // The channel closes here: a writer that did not get `Complete` frees
    // the range again.
    drop(tx);
    let written = worker.join().await;
    let result = outcome.and(written);
    if result.is_err() && access.check_write().is_err() {
        // The right to write is gone: the stage goes as well.
        stages.discard(&request.stage);
    }
    match result {
        Ok(()) => reply(&mut send, FsResponse::Ok).await,
        Err(error) => reply_err(&mut send, error).await,
    }
}

/// Checks the right again and enters the range as taken.
fn admit_range(
    access: &FsAccess,
    stages: &Arc<RangeStages>,
    stage: &str,
    offset: u64,
    len: u64,
) -> io::Result<Claim> {
    if let Err(error) = access.resolve_write(stage) {
        stages.discard(stage);
        return Err(error);
    }
    stages.claim(stage, offset, len)
}

/// Reads the content frames of one range: exactly `len` raw bytes, then the
/// end of the sender's stream. Fewer bytes, more bytes, a control frame or a
/// block that does not unpack is an error and leaves the range uncovered.
async fn receive_range(
    recv: &mut iroh::endpoint::RecvStream,
    tx: &mpsc::Sender<Command>,
    len: u64,
    lz4: bool,
    stall: std::time::Duration,
) -> io::Result<()> {
    let mut received = 0u64;
    loop {
        let frame = io_deadline::run_for(
            "Share stage write data",
            stall,
            recv_content_frame_or_end(recv, lz4),
        )
        .await?;
        let Some((tag, payload)) = frame else {
            if received == len {
                return Ok(());
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("{received} statt {len} Bytes für den Bereich empfangen"),
            ));
        };
        let bytes = match classify(tag, payload, lz4)? {
            Content::Bytes(bytes) => bytes,
            Content::Other(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Unerwarteter Steuerframe im Bereichsstrom",
                ))
            }
        };
        received = received.saturating_add(bytes.len() as u64);
        if received > len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Mehr als die angekündigten {len} Bytes für den Bereich"),
            ));
        }
        if tx.send(Command::Data(bytes)).await.is_err() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Der Bereichsschreiber ist beendet",
            ));
        }
    }
}

/// Writes the chunks of one range through the stage's handle. The right to
/// write is checked again before every chunk and before the range counts as
/// finished.
fn write_range(
    mut claim: Claim,
    access: FsAccess,
    mut commands: mpsc::Receiver<Command>,
) -> io::Result<()> {
    while let Some(command) = commands.blocking_recv() {
        match command {
            Command::Data(bytes) => {
                access.check_write()?;
                claim.write(&bytes)?;
            }
            Command::Complete => {
                access.check_write()?;
                return claim.complete();
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::ConnectionAborted,
        "Bereich wurde abgebrochen",
    ))
}

/// `SegmentedStageFinish`: every range arrived, the stage has its length.
pub(super) async fn finish(stream: FsStream, request: FsSegmentedFinish) -> io::Result<()> {
    let FsStream {
        mut send,
        access,
        authorization,
        context,
        principal,
        ..
    } = stream;
    let stages = context.stages.clone();
    let result = control(&principal, "Share finish range stage", move || {
        run_authorized(authorization.as_ref(), || {
            if let Err(error) = access.resolve_write(&request.stage) {
                stages.discard(&request.stage);
                return Err(error);
            }
            stages.finish(&request.stage)
        })
    })
    .await;
    super::control_operations::reply_unit(&mut send, result).await
}

/// `DiscardStage` for a stage this connection still holds open: the file and
/// the table entry go.
pub(super) async fn discard(
    mut send: iroh::endpoint::SendStream,
    path: String,
    access: FsAccess,
    authorization: Option<MountLeaseAuthorization>,
    context: &super::StreamContext,
    principal: &PeerPrincipal,
) -> io::Result<()> {
    let stages = context.stages.clone();
    let result = control(principal, "Share discard range stage", move || {
        run_authorized(authorization.as_ref(), || {
            access.resolve_write(&path)?;
            stages.discard(&path);
            Ok(())
        })
    })
    .await;
    super::control_operations::reply_unit(&mut send, result).await
}
