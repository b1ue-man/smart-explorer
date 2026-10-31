//! The app's side of the worker's job service (Z-P2, 2026-10-28): the
//! desktop app, `se` and `se mcp` hand a transfer job with a Share side to
//! the worker that owns the Share node, follow its progress and issues, and
//! cancel it there. No byte of such a job passes the loopback proxy any
//! more, and the Share host's evidence reaches the engine directly.
//!
//! A job goes to the worker only when every remote side is a Share device
//! this process opened through the worker (`remember_share`), the worker
//! announces the service, and it accepts the job for this account. Before
//! the worker said `Started`, nothing runs there, so every refusal keeps
//! today's path through the proxy. After it, the job is the worker's: a
//! lost connection is reported, never answered by running the job again.
use super::ipc_protocol::transfer::{
    EndpointWire, JobWire, TransferIpcReply, TransferIpcRequest, TRANSFER_JOBS_V1,
    TRANSFER_UPDATE_V1,
};
use super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse,
};
use crate::share::PeerOpenTarget;
use crate::transfer::{JobView, Side, TransferIssue, TransferMsg, TransferProgress};
use crate::vfs::{Backend, BackendHandle};
use std::io::{self, BufRead, BufReader, Read};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

/// Wait for the worker's `Started`: it opens each Share device first (up to
/// 45 s of attempts and one handshake, `ipc_open_client.rs`).
const START_TIMEOUT: Duration = Duration::from_secs(120);
/// Every look at the cancel flag while the job runs.
const POLL: Duration = Duration::from_millis(200);
/// The worker sends at least every 5 s; this much silence is a lost worker.
const SILENCE: Duration = Duration::from_secs(60);
const CONNECT: Duration = Duration::from_secs(2);

/// Live handles (with their thin data address) and the device each reaches.
type Shares = Vec<(Weak<dyn Backend>, usize, PeerOpenTarget)>;

static SHARES: Mutex<Shares> = Mutex::new(Vec::new());

fn shares() -> MutexGuard<'static, Shares> {
    SHARES.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Remembers that `handle` reaches `target` through the worker
/// (`open_share_backend`); entries end with their handle.
pub(crate) fn remember_share(handle: &BackendHandle, target: &PeerOpenTarget) {
    let mut shares = shares();
    shares.retain(|(backend, _, _)| backend.strong_count() > 0);
    let address = crate::transfer::address_of(&**handle);
    shares.push((Arc::downgrade(handle), address, target.clone()));
}

fn lookup(address: usize) -> Option<PeerOpenTarget> {
    shares()
        .iter()
        .find(|(backend, at, _)| backend.strong_count() > 0 && *at == address)
        .map(|(_, _, target)| target.clone())
}

/// The Share device `backend` (or the live backend of its browsing cache)
/// reaches through the worker.
fn share_target(backend: &dyn Backend) -> Option<PeerOpenTarget> {
    if let Some(target) = lookup(crate::transfer::address_of(backend)) {
        return Some(target);
    }
    let inner = backend.uncached_backend()?;
    lookup(crate::transfer::address_of(&*inner))
}

/// The wire form of a side; `None` for a remote side the worker does not
/// reach itself.
fn endpoint(side: Side<'_>) -> Option<EndpointWire> {
    match side {
        Side::Local => Some(EndpointWire::Local),
        Side::Remote(backend) => share_target(backend).map(|target| EndpointWire::Share {
            target,
            namespace: backend.namespace_identity(),
            state: backend.state_identity(),
        }),
    }
}

/// Runs `view` in the worker when it qualifies; `false` when nothing was
/// started there and the caller runs the job itself. `true` means `report`
/// received exactly one `Done`.
pub(crate) fn run_transfer_in_worker(
    view: &JobView<'_>,
    report: &(dyn Fn(TransferMsg) + Sync),
    cancel: &AtomicBool,
) -> bool {
    if shares().is_empty() || (view.source.is_local() && view.target.is_local()) {
        return false;
    }
    let (Some(source), Some(target)) = (endpoint(view.source), endpoint(view.target)) else {
        return false;
    };
    if cancel.load(Ordering::Acquire) {
        return false;
    }
    let job = JobWire::of(view, source, target);
    let Some((address, token)) = worker() else {
        return false;
    };
    if !offers_jobs(address, &token, job.needs_update_rule()) {
        return false;
    }
    match start(address, &token, job) {
        Start::NotHere => false,
        Start::Failed(message) => {
            report(failure(view, message, cancel.load(Ordering::Acquire)));
            true
        }
        Start::Running { stream, id } => {
            follow(
                view,
                report,
                cancel,
                stream,
                (address, token.as_str(), id.as_str()),
            );
            true
        }
    }
}

fn worker() -> Option<(SocketAddr, String)> {
    let address = super::ipc_storage::read_ipc_addr()?;
    let token = super::ipc_storage::read_token().ok()?;
    Some((address, token))
}

/// One request on a new connection and its first answer.
fn ask(
    address: SocketAddr,
    token: &str,
    req: TransferIpcRequest,
    timeout: Duration,
) -> io::Result<(TcpStream, IpcResponse)> {
    let mut stream = TcpStream::connect_timeout(&address, CONNECT)?;
    set_stream_timeout(&stream, Some(timeout));
    let request = IpcRequest::Transfer {
        token: token.to_string(),
        req,
    };
    write_request(&mut stream, &request)?;
    let response = read_response(&mut stream)?;
    Ok((stream, response))
}

/// Whether the worker announces the job service (its hello) and, for a job
/// that uses the file rule "neuere ersetzen ältere", knows that rule.
fn offers_jobs(address: SocketAddr, token: &str, update_rule: bool) -> bool {
    matches!(
        ask(address, token, TransferIpcRequest::Hello, Duration::from_secs(5)),
        Ok((_, IpcResponse::Transfer(TransferIpcReply::Hello { capabilities })))
            if capabilities.iter().any(|label| label == TRANSFER_JOBS_V1)
                && (!update_rule || capabilities.iter().any(|label| label == TRANSFER_UPDATE_V1))
    )
}

enum Start {
    /// Nothing runs in the worker: the caller keeps the proxy path.
    NotHere,
    /// The worker answered with a reason, or the answer is unknown: the job
    /// is not started again here.
    Failed(String),
    Running {
        stream: TcpStream,
        id: String,
    },
}

fn start(address: SocketAddr, token: &str, job: JobWire) -> Start {
    let id = job.id.clone();
    let mut stream = match TcpStream::connect_timeout(&address, CONNECT) {
        Ok(stream) => stream,
        Err(_) => return Start::NotHere,
    };
    set_stream_timeout(&stream, Some(START_TIMEOUT));
    let request = IpcRequest::Transfer {
        token: token.to_string(),
        req: TransferIpcRequest::Start { job: Box::new(job) },
    };
    // A job too large for one IPC line (a long explicit file list) is
    // refused before anything is sent.
    if write_request(&mut stream, &request).is_err() {
        return Start::NotHere;
    }
    match read_response(&mut stream) {
        Ok(IpcResponse::Transfer(TransferIpcReply::Started { id })) => {
            Start::Running { stream, id }
        }
        Ok(IpcResponse::Err { code, .. })
            if code.as_deref() == Some(super::transfer_job_ipc::NOT_HERE_CODE) =>
        {
            Start::NotHere
        }
        Ok(IpcResponse::Err { msg, .. }) => Start::Failed(msg),
        Ok(_) => Start::Failed("Unerwartete Antwort des Hintergrund-Workers".into()),
        Err(error) => {
            // The worker may have started it: stop it rather than run it twice.
            if let Some(id) = id {
                send_cancel(address, token, &id);
            }
            Start::Failed(format!(
                "Hintergrund-Worker hat den Start der Übertragung nicht bestätigt: {error}"
            ))
        }
    }
}

fn send_cancel(address: SocketAddr, token: &str, id: &str) {
    let _ = ask(
        address,
        token,
        TransferIpcRequest::Cancel { id: id.to_string() },
        Duration::from_secs(5),
    );
}

/// Follows the running job: progress and its end go to `report`; a cancel
/// goes to the worker once.
fn follow(
    view: &JobView<'_>,
    report: &(dyn Fn(TransferMsg) + Sync),
    cancel: &AtomicBool,
    stream: TcpStream,
    (address, token, id): (SocketAddr, &str, &str),
) {
    set_stream_timeout(&stream, Some(POLL));
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    let mut heard = Instant::now();
    let mut cancel_sent = false;
    let lost = loop {
        if !cancel_sent && cancel.load(Ordering::Acquire) {
            cancel_sent = true;
            send_cancel(address, token, id);
        }
        match read_piece(&mut reader, &mut line) {
            Ok(true) => {
                heard = Instant::now();
                let response = serde_json::from_slice::<IpcResponse>(&line);
                line.clear();
                match response {
                    Ok(IpcResponse::Transfer(TransferIpcReply::Progress { progress })) => {
                        report(TransferMsg::Progress(*progress));
                    }
                    Ok(IpcResponse::Transfer(TransferIpcReply::Done {
                        progress,
                        errors,
                        canceled,
                        issues,
                        roots,
                    })) => {
                        refresh_caches(view);
                        report(TransferMsg::Done {
                            progress: *progress,
                            errors,
                            canceled,
                            issues,
                            roots,
                        });
                        return;
                    }
                    Ok(_) => break "unerwartete Antwort".to_string(),
                    Err(error) => break error.to_string(),
                }
            }
            Ok(false) => break "Verbindung beendet".to_string(),
            Err(error) if waiting(&error) => {
                if heard.elapsed() >= SILENCE {
                    break format!("{} s ohne Nachricht", SILENCE.as_secs());
                }
            }
            Err(error) => break error.to_string(),
        }
    };
    refresh_caches(view);
    let message = format!(
        "Verbindung zum Hintergrund-Worker verloren ({lost}); die Übertragung kann dort weiterlaufen oder beendet sein – „Fehlende übertragen“ setzt sie fort, ohne Vorhandenes doppelt anzulegen"
    );
    report(failure(view, message, cancel.load(Ordering::Acquire)));
}

/// A read timeout: `WouldBlock` on Unix, `TimedOut` on Windows.
fn waiting(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// Reads on in `line`; `Ok(true)` once it holds a whole line, `Ok(false)`
/// at the end of the stream. Bytes read before a timeout stay in `line`
/// (`read_until` keeps them), so the next call continues the same line.
fn read_piece(reader: &mut BufReader<TcpStream>, line: &mut Vec<u8>) -> io::Result<bool> {
    loop {
        let room = super::line::MAX_IPC_LINE.saturating_sub(line.len()) as u64;
        if room == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ipc line too large",
            ));
        }
        let read = reader.by_ref().take(room).read_until(b'\n', line)?;
        if line.last() == Some(&b'\n') {
            return Ok(true);
        }
        if read == 0 {
            return Ok(false);
        }
    }
}

/// The worker wrote at the job's places: browsing caches of this process
/// show them fresh.
fn refresh_caches(view: &JobView<'_>) {
    for side in [view.source, view.target] {
        if let Some(backend) = side.backend() {
            backend.invalidate_cache();
        }
    }
}

/// The terminal message of a job that did not report its own end here.
fn failure(view: &JobView<'_>, message: String, canceled: bool) -> TransferMsg {
    let kind = view.kind();
    let mut progress = TransferProgress::new(kind, kind.label(), 0, 0);
    progress.source = view.source_label.to_string();
    progress.target = view.target_label.to_string();
    progress.done = true;
    progress.errors = 1;
    TransferMsg::Done {
        progress,
        errors: vec![message.clone()],
        canceled,
        issues: vec![TransferIssue {
            path: String::new(),
            message,
        }],
        roots: Vec::new(),
    }
}

/// Opens `target` with the embedded worker's own Share host in this process
/// (Android, Z-A1): no loopback proxy into the same process. Jobs on the
/// backend record the identity the proxy handle reports, so records stay
/// exchangeable. `None` without an embedded worker that serves.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) fn open_share_in_process(
    target: &PeerOpenTarget,
) -> Option<Result<(String, BackendHandle, crate::share::ShareStatus), String>> {
    let host = super::live::embedded_host()?;
    Some(
        host.open_share(target.clone())
            .map(|(label, backend, status)| {
                let proxy =
                    super::ipc_client::share_backend_identity(label.clone(), target.clone());
                crate::transfer::remember_identity(
                    &backend,
                    proxy.namespace_identity(),
                    proxy.state_identity(),
                );
                (label, backend, status)
            }),
    )
}

#[cfg(test)]
#[path = "transfer_job_client_tests.rs"]
mod tests;
