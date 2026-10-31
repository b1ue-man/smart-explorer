//! The worker's job service (Z-P2, 2026-10-28): a transfer job with a Share
//! side runs in this worker, next to the Share node, instead of in the app
//! with every byte through the worker's loopback proxy. Called only after
//! the shared daemon-token check of the IPC router.
//!
//! - Same person: a job reads and writes local files with this worker's
//!   rights, so it starts only for a connection of exactly this worker's
//!   account (no seat-user exception); anything else keeps the proxy path.
//! - Crash safety is the engine's: its crash log and receipts live in the
//!   same app data folder, written by this process. A job with a crash log
//!   keeps running when its client goes away (a crashed app); every other
//!   job ends with its client, as it ended with the app before.
//! - One run per job id at a time; `Cancel` sets the job's stop flag.
use super::ipc_host::ShareHost;
use super::ipc_protocol::transfer::{
    EndpointWire, JobWire, TransferIpcReply, TransferIpcRequest, TRANSFER_JOBS_V1,
    TRANSFER_UPDATE_V1,
};
use super::ipc_protocol::{write_response, IpcResponse};
use crate::transfer::{JobView, Side, TransferIssue, TransferMsg, TransferProgress};
use crate::types::{Conflict, FilterDef};
use crate::vfs::BackendHandle;
use std::io;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// Code of a refusal after which the client keeps the proxy path (nothing
/// was started).
pub(super) const NOT_HERE_CODE: &str = "transfer_not_here";
/// Without any message for this long, the last progress goes out again, so
/// the client can tell a working job from a lost worker.
const KEEPALIVE: Duration = Duration::from_secs(5);
/// A write to the client that blocks this long means the client is gone.
const CLIENT_WRITE_LIMIT: Duration = Duration::from_secs(30);
/// Room a terminal line keeps below the IPC line limit.
const LINE_BUDGET: usize = super::line::MAX_IPC_LINE - 64 * 1024;

/// Running jobs by id with their stop flags.
static RUNNING: Mutex<Vec<(String, Arc<AtomicBool>)>> = Mutex::new(Vec::new());

fn running() -> MutexGuard<'static, Vec<(String, Arc<AtomicBool>)>> {
    RUNNING.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The registration of one running job; removed when the job ends.
struct Registered(String);

impl Registered {
    fn new(id: &str, stop: Arc<AtomicBool>) -> Option<Self> {
        let mut jobs = running();
        if jobs.iter().any(|(running, _)| running == id) {
            return None;
        }
        jobs.push((id.to_string(), stop));
        Some(Self(id.to_string()))
    }
}

impl Drop for Registered {
    fn drop(&mut self) {
        running().retain(|(id, _)| *id != self.0);
    }
}

fn cancel(id: &str) -> bool {
    let jobs = running();
    match jobs.iter().find(|(running, _)| running == id) {
        Some((_, stop)) => {
            stop.store(true, Ordering::Release);
            true
        }
        None => false,
    }
}

/// A job with its live endpoints, ready to run.
struct OpenedJob {
    wire: JobWire,
    conflict: Conflict,
    filter: Option<(FilterDef, String)>,
    source: Option<BackendHandle>,
    target: Option<BackendHandle>,
}

fn side(backend: &Option<BackendHandle>) -> Side<'_> {
    match backend {
        Some(backend) => Side::Remote(&**backend),
        None => Side::Local,
    }
}

impl OpenedJob {
    fn view(&self) -> JobView<'_> {
        JobView {
            id: self.wire.id.as_deref(),
            recoverable: self.wire.recoverable,
            recovery: self.wire.recovery(),
            source: side(&self.source),
            target: side(&self.target),
            target_dir: &self.wire.target_dir,
            items: &self.wire.items,
            layout: self.wire.layout,
            filter: self.filter.as_ref(),
            conflict: self.conflict,
            folders: &self.wire.folders,
            mode: self.wire.mode(),
            source_label: &self.wire.source_label,
            target_label: &self.wire.target_label,
            resume: self.wire.resume.as_deref(),
        }
    }
}

impl ShareHost {
    pub(in crate::daemon) fn serve_transfer(
        &self,
        mut stream: TcpStream,
        request: TransferIpcRequest,
        lease: Option<Arc<crate::mcp_launcher::OperationGuard>>,
    ) -> io::Result<()> {
        let reply = match request {
            TransferIpcRequest::Hello => TransferIpcReply::Hello {
                capabilities: vec![TRANSFER_JOBS_V1.to_string(), TRANSFER_UPDATE_V1.to_string()],
            },
            TransferIpcRequest::Cancel { id } => TransferIpcReply::Cancelled { found: cancel(&id) },
            TransferIpcRequest::Start { job } => {
                // An update waits for the job like for any other request.
                let _lease = lease;
                if let Err(message) = same_account(&stream) {
                    return refuse(&mut stream, message, NOT_HERE_CODE);
                }
                return match self.open_job(*job) {
                    Ok(job) => run(stream, job),
                    Err((message, code)) => refuse(&mut stream, message, code),
                };
            }
        };
        write_response(&mut stream, &IpcResponse::Transfer(reply))
    }

    /// Opens the job's Share devices in this worker. Both sides on one
    /// device share one backend, so server copies and the namespace checks
    /// see one account as before.
    fn open_job(&self, wire: JobWire) -> Result<OpenedJob, (String, &'static str)> {
        let (conflict, filter) = wire
            .decoded()
            .map_err(|message| (message, "transfer_invalid"))?;
        let source = self.open_side(&wire.source, None)?;
        let target = self.open_side(&wire.target, Some((&wire.source, &source)))?;
        Ok(OpenedJob {
            wire,
            conflict,
            filter,
            source,
            target,
        })
    }

    fn open_side(
        &self,
        side: &EndpointWire,
        other: Option<(&EndpointWire, &Option<BackendHandle>)>,
    ) -> Result<Option<BackendHandle>, (String, &'static str)> {
        let EndpointWire::Share {
            target,
            namespace,
            state,
        } = side
        else {
            return Ok(None);
        };
        // The identity is the one the app's proxy handle reports for a
        // Share device (`Peer:<label>`); nothing else is recorded here.
        if !state.starts_with("Peer:") || !namespace.starts_with("Peer:") {
            return Err((
                "Unerwartete Identität einer Share-Seite".into(),
                NOT_HERE_CODE,
            ));
        }
        if let Some((EndpointWire::Share { target: open, .. }, Some(backend))) = other {
            if open == target {
                return Ok(Some(backend.clone()));
            }
        }
        let (_, backend, _) = self
            .open_share(target.clone())
            .map_err(|message| (message, "transfer_open_failed"))?;
        crate::transfer::remember_identity(&backend, namespace.clone(), state.clone());
        Ok(Some(backend))
    }
}

/// Only a connection of exactly this worker's account may run a job here.
fn same_account(stream: &TcpStream) -> Result<(), String> {
    let peer = stream.peer_addr().map_err(|error| error.to_string())?;
    let local = stream.local_addr().map_err(|error| error.to_string())?;
    let own = crate::share::os_sockets::own_account().map_err(|error| error.to_string())?;
    match crate::share::os_sockets::connection_account(peer, local) {
        Ok(Some(account)) if account == own => Ok(()),
        Ok(Some(_)) => Err("Auftrag eines anderen Kontos: bleibt beim Proxyweg".into()),
        Ok(None) | Err(_) => Err("Konto des Auftraggebers nicht ermittelbar".into()),
    }
}

fn refuse(stream: &mut TcpStream, msg: String, code: &str) -> io::Result<()> {
    write_response(
        stream,
        &IpcResponse::Err {
            msg,
            code: Some(code.to_string()),
        },
    )
}

/// Runs `job` on a thread of its own and forwards its messages to the
/// client while it listens; returns once the job reported its end.
fn run(mut stream: TcpStream, job: OpenedJob) -> io::Result<()> {
    let id = job
        .wire
        .id
        .clone()
        .unwrap_or_else(crate::transfer::new_job_id);
    let stop = Arc::new(AtomicBool::new(false));
    let Some(_registered) = Registered::new(&id, stop.clone()) else {
        return refuse(
            &mut stream,
            format!("Auftrag {id} läuft bereits im Hintergrund-Worker"),
            "transfer_running",
        );
    };
    let survives_client = job.wire.survives_client;
    let kind = job.view().kind();
    // A client that stops reading for this long counts as gone; reading
    // needs no limit (the client sends nothing more on this stream).
    let _ = stream.set_read_timeout(None);
    let _ = stream.set_write_timeout(Some(CLIENT_WRITE_LIMIT));
    write_response(
        &mut stream,
        &IpcResponse::Transfer(TransferIpcReply::Started { id: id.clone() }),
    )?;
    let (tx, rx) = crossbeam_channel::unbounded::<TransferMsg>();
    let job_stop = stop.clone();
    let spawned = std::thread::Builder::new()
        .name("transfer-job".into())
        .spawn(move || {
            let report = move |message: TransferMsg| {
                let _ = tx.send(message);
            };
            crate::transfer::run_view_here(job.view(), &report, &job_stop);
        });
    let worker = match spawned {
        Ok(worker) => worker,
        Err(error) => {
            let message = format!("Übertragung konnte nicht gestartet werden: {error}");
            return write_reply(&mut stream, failed_done(kind, message, false));
        }
    };
    let mut client = Some(stream);
    let mut last: Option<TransferProgress> = None;
    let mut written = Instant::now();
    loop {
        let reply = match rx.recv_timeout(KEEPALIVE) {
            Ok(TransferMsg::Progress(progress)) => {
                last = Some(progress.clone());
                TransferIpcReply::Progress {
                    progress: Box::new(progress),
                }
            }
            Ok(TransferMsg::Done {
                progress,
                errors,
                canceled,
                issues,
                roots,
            }) => {
                let done = bounded_done(progress, errors, canceled, issues, roots);
                if let Some(stream) = client.as_mut() {
                    let _ = write_reply(stream, done);
                }
                break;
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => match &last {
                Some(progress) if written.elapsed() >= KEEPALIVE => TransferIpcReply::Progress {
                    progress: Box::new(progress.clone()),
                },
                _ => continue,
            },
            // The job thread ended without its terminal message (a panic).
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                let canceled = stop.load(Ordering::Acquire);
                let message = "Übertragung im Hintergrund-Worker abgebrochen".to_string();
                let done = failed_done(kind, message, canceled);
                if let Some(stream) = client.as_mut() {
                    let _ = write_reply(stream, done);
                }
                break;
            }
        };
        let Some(stream) = client.as_mut() else {
            continue;
        };
        if write_reply(stream, reply).is_err() {
            // The client is gone. A job with a crash log goes on (its end
            // is in the log and the recovery list); any other job stops.
            client = None;
            if !survives_client {
                stop.store(true, Ordering::Release);
            }
        } else {
            written = Instant::now();
        }
    }
    let _ = worker.join();
    Ok(())
}

fn write_reply(stream: &mut TcpStream, reply: TransferIpcReply) -> io::Result<()> {
    write_response(stream, &IpcResponse::Transfer(reply))
}

/// The terminal message within the IPC line limit: the shown issues are
/// halved until it fits (the job's log keeps every issue).
fn bounded_done(
    progress: TransferProgress,
    mut errors: Vec<String>,
    canceled: bool,
    mut issues: Vec<TransferIssue>,
    roots: Vec<crate::transfer::ResolvedRoot>,
) -> TransferIpcReply {
    let progress = Box::new(progress);
    loop {
        let reply = TransferIpcReply::Done {
            progress: progress.clone(),
            errors: errors.clone(),
            canceled,
            issues: issues.clone(),
            roots: roots.clone(),
        };
        let fits = serde_json::to_string(&reply).is_ok_and(|line| line.len() <= LINE_BUDGET);
        if fits || (issues.is_empty() && errors.is_empty()) {
            return reply;
        }
        issues.truncate(issues.len() / 2);
        errors.truncate(errors.len() / 2);
    }
}

fn failed_done(
    kind: crate::transfer::TransferKind,
    message: String,
    canceled: bool,
) -> TransferIpcReply {
    let mut progress = TransferProgress::new(kind, kind.label(), 0, 0);
    progress.done = true;
    progress.errors = 1;
    TransferIpcReply::Done {
        progress: Box::new(progress),
        errors: vec![message.clone()],
        canceled,
        issues: vec![TransferIssue {
            path: String::new(),
            message,
        }],
        roots: Vec::new(),
    }
}

#[cfg(test)]
#[path = "transfer_job_speed_task_tests.rs"]
mod speed_task_tests;
#[cfg(test)]
#[path = "transfer_job_ipc_tests.rs"]
mod tests;
