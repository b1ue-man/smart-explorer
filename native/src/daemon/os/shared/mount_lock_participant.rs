//! The lock participant of vault drives and both ends of their lock
//! channel (DRV-M7; S2-V4, S2-V10, C6 of the drive strand, C3.3 of the vault
//! strand).
//!
//! Worker side: one participant of the session register handles every
//! vault drive of the locking site. It posts an order to each running drive
//! of the site at once, then waits for their answers until the deadline of
//! the lock (sleep 4 s, explicit lock and removal 60 s, otherwise 15 s). A
//! drive that unmounted is waited for until its host ended; a drive that
//! keeps unpublished changes refuses a lock someone asked for without
//! „Änderungen verschlüsselt aufbewahren“, and the participant returns
//! `Err` with what stays unsaved; a drive that misses the deadline is
//! aborted (Linux: lazy unmount and `fusectl` abort, then the host ends;
//! Windows: the host ends in its job object). Its journal keeps the sealed
//! changes. The participant confirms only after every drive is gone, so no
//! drive answers a read or a write after the key is wiped.
//!
//! Host side: `HostLockWatch` asks the worker for orders over
//! `MountHostExt` (`PrepareLock`), hands them to the control loop of the
//! host and writes the loop's answer back on the same connection.
use std::collections::HashMap;
use std::io;
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::mount::os::shared::host_session::{LockAnswer, LockOrder};
use crate::mount::MountId;
use crate::vault::{LockParticipant, LockRequest};

use super::super::drive_modules::ipc_protocol_mount_host::{
    LockPrepared, MountHostReply, MountHostRequest, MAX_PREPARED_LINE, MAX_WATCH_MS,
};
use super::super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, write_response, IpcRequest, IpcResponse,
};
use super::super::line::read_line_limited_from_stream;
use super::MountManager;

/// One wait of the host's lock watch.
const WATCH_MS: u64 = 25_000;
/// How often the participant looks for the end of an unmounted host.
const EXIT_POLL: Duration = Duration::from_millis(50);

struct Order {
    deadline: Instant,
    keep_unsaved: bool,
    taken: bool,
    answer: Option<LockPrepared>,
}

/// Lock orders of this worker's vault drives, by mount id.
#[derive(Default)]
pub(super) struct LockDesk {
    orders: Mutex<HashMap<String, Order>>,
    changed: Condvar,
}

impl LockDesk {
    fn orders(&self) -> MutexGuard<'_, HashMap<String, Order>> {
        self.orders.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Posts the order for drive `id`; an older one is replaced.
    pub(super) fn post(&self, id: &str, deadline: Instant, keep_unsaved: bool) {
        let order = Order {
            deadline,
            keep_unsaved,
            taken: false,
            answer: None,
        };
        self.orders().insert(id.to_string(), order);
        self.changed.notify_all();
    }

    /// The host of `id` waits up to `wait` for an order and takes it.
    pub(super) fn take(&self, id: &str, wait: Duration) -> Option<(Instant, bool)> {
        let until = Instant::now() + wait;
        let mut orders = self.orders();
        loop {
            if let Some(order) = orders.get_mut(id).filter(|order| !order.taken) {
                order.taken = true;
                return Some((order.deadline, order.keep_unsaved));
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            orders = self
                .changed
                .wait_timeout(orders, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// The host's answer to the order of `id`.
    pub(super) fn answer(&self, id: &str, answer: LockPrepared) {
        if let Some(order) = self.orders().get_mut(id) {
            order.answer = Some(answer);
        }
        self.changed.notify_all();
    }

    /// Waits until `deadline` for the answer of `id`; the order goes.
    pub(super) fn wait_answer(&self, id: &str, deadline: Instant) -> Option<LockPrepared> {
        let mut orders = self.orders();
        loop {
            if let Some(answer) = orders.get_mut(id).and_then(|order| order.answer.take()) {
                orders.remove(id);
                return Some(answer);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                orders.remove(id);
                return None;
            }
            orders = self
                .changed
                .wait_timeout(orders, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

/// What a lock does with one drive after its answer (or its silence).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DriveVerdict {
    /// The volume is gone; the host ends now.
    Unmounted,
    /// The drive keeps unpublished changes and stays mounted.
    Refused(String),
    /// No answer by the deadline, or a refusal a forced lock cannot accept.
    Abort,
}

pub(super) fn verdict(answer: Option<&LockPrepared>, keep_unsaved: bool) -> DriveVerdict {
    match answer {
        None => DriveVerdict::Abort,
        Some(answer) if answer.unmounted => DriveVerdict::Unmounted,
        Some(_) if keep_unsaved => DriveVerdict::Abort,
        Some(answer) => {
            let failed = if answer.detail.is_some() {
                "; Veröffentlichen gescheitert"
            } else {
                ""
            };
            DriveVerdict::Refused(format!(
                "{} Änderung(en) nicht veröffentlicht{failed}",
                answer.pending
            ))
        }
    }
}

/// The participant of every vault drive of this worker.
pub(super) struct MountLockParticipant {
    pub(super) manager: MountManager,
}

impl LockParticipant for MountLockParticipant {
    fn prepare_lock(&self, site: &str, request: &LockRequest) -> Result<(), String> {
        let keep = request.keep_unsaved || request.reason.forced();
        self.manager
            .prepare_vault_lock(site, request.deadline, keep)
    }
}

impl MountManager {
    /// Ends every running drive of `site` for its lock (module docs).
    pub(super) fn prepare_vault_lock(
        &self,
        site: &str,
        deadline: Instant,
        keep_unsaved: bool,
    ) -> Result<(), String> {
        let drives: Vec<(String, String, Option<PathBuf>)> = {
            let mut state = self.state_guard()?;
            state
                .iter_mut()
                .filter(|(_, entry)| entry.child.is_some())
                .filter(|(_, entry)| super::termination::vault_site(&entry.config) == Some(site))
                .map(|(key, entry)| {
                    entry.lock_end = true;
                    let folder = entry.config.location_directory().map(PathBuf::from);
                    (key.clone(), entry.config.label.clone(), folder)
                })
                .collect()
        };
        for (key, _, _) in &drives {
            self.vault.desk.post(key, deadline, keep_unsaved);
        }
        let mut refused = Vec::new();
        for (key, label, folder) in &drives {
            let answer = self.vault.desk.wait_answer(key, deadline);
            match verdict(answer.as_ref(), keep_unsaved) {
                DriveVerdict::Unmounted => {
                    let pending = answer.map_or(0, |answer| answer.pending);
                    self.note_lock_pending(key, pending);
                    if !self.wait_exit(key, deadline) {
                        self.abort_drive(key, folder.as_ref());
                    }
                }
                DriveVerdict::Refused(text) => {
                    if let Ok(mut state) = self.state_guard() {
                        if let Some(entry) = state.get_mut(key) {
                            entry.lock_end = false;
                        }
                    }
                    refused.push(format!("Laufwerk „{label}“: {text}"));
                }
                DriveVerdict::Abort => self.abort_drive(key, folder.as_ref()),
            }
        }
        if refused.is_empty() {
            Ok(())
        } else {
            Err(refused.join("; "))
        }
    }

    fn note_lock_pending(&self, key: &str, pending: u64) {
        if let Ok(mut state) = self.state_guard() {
            if let Some(entry) = state.get_mut(key) {
                entry.lock_pending = Some(u32::try_from(pending).unwrap_or(u32::MAX));
            }
        }
    }

    /// Waits until the host of `key` ended and its exit is booked.
    fn wait_exit(&self, key: &str, deadline: Instant) -> bool {
        loop {
            match self.observe_exit(key) {
                Ok(Some(_)) | Err(_) => return true,
                Ok(None) => {}
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(EXIT_POLL);
        }
    }

    /// Cuts a drive that did not end in time; its journal stays sealed.
    fn abort_drive(&self, key: &str, folder: Option<&PathBuf>) {
        #[cfg(target_os = "linux")]
        if let Some(folder) = folder {
            let _ = crate::mount::unmount_drive_folder(folder, true);
        }
        #[cfg(not(target_os = "linux"))]
        let _ = folder;
        let _ = self.force_stop(key);
    }

    /// `IpcRequest::MountHostExt` of the host of `id`, after its session
    /// token was checked (`ipc_host_ext.rs`).
    pub(in crate::daemon) fn serve_host_ext(
        &self,
        mut stream: TcpStream,
        id: &MountId,
        req: MountHostRequest,
    ) -> io::Result<()> {
        let MountHostRequest::PrepareLock { deadline_ms } = req;
        let is_vault = self
            .state_guard()
            .ok()
            .and_then(|state| {
                state
                    .get(id.as_str())
                    .map(|entry| super::termination::vault_site(&entry.config).is_some())
            })
            .unwrap_or(false);
        if !is_vault {
            let msg = "PrepareLock gilt nur für Tresor-Laufwerke".to_string();
            return write_response(&mut stream, &IpcResponse::Err { msg, code: None });
        }
        let wait = Duration::from_millis(deadline_ms.min(MAX_WATCH_MS));
        let Some((deadline, keep_unsaved)) = self.vault.desk.take(id.as_str(), wait) else {
            return write_response(
                &mut stream,
                &IpcResponse::MountHostExt(MountHostReply::Idle),
            );
        };
        let left = deadline.saturating_duration_since(Instant::now());
        let reply = MountHostReply::PrepareLock {
            deadline_ms: millis(left),
            keep_unsaved,
        };
        write_response(&mut stream, &IpcResponse::MountHostExt(reply))?;
        set_stream_timeout(&stream, Some(left + Duration::from_secs(1)));
        let mut line = String::new();
        read_line_limited_from_stream(&mut stream, &mut line, MAX_PREPARED_LINE)?;
        let answer: LockPrepared = serde_json::from_str(line.trim()).map_err(io::Error::other)?;
        self.vault.desk.answer(id.as_str(), answer);
        Ok(())
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[derive(Default)]
struct WatchSlot {
    order: Option<LockOrder>,
    reply: Option<mpsc::Sender<LockAnswer>>,
}

/// The host side of the lock channel of one vault drive.
pub(in crate::daemon) struct HostLockWatch {
    slot: Mutex<WatchSlot>,
}

/// How one round of the watch ended.
enum Round {
    Again,
    Unmounted,
}

impl HostLockWatch {
    /// Starts the watch thread of the drive `id`.
    pub(in crate::daemon) fn start(addr: SocketAddr, id: MountId, token: String) -> Arc<Self> {
        let watch = Arc::new(Self {
            slot: Mutex::new(WatchSlot::default()),
        });
        let thread_watch = Arc::clone(&watch);
        let spawned = std::thread::Builder::new()
            .name(format!("mount-lock-{}", id.as_str()))
            .spawn(move || thread_watch.run(addr, &id, &token));
        if let Err(error) = spawned {
            eprintln!("mount lock watch not started: {error}");
        }
        watch
    }

    fn slot(&self) -> MutexGuard<'_, WatchSlot> {
        self.slot.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn run(&self, addr: SocketAddr, id: &MountId, token: &str) {
        loop {
            match self.round(addr, id, token) {
                Ok(Round::Again) => {}
                Ok(Round::Unmounted) => return,
                // A worker that restarts or a drive that ends: the control
                // stream decides about the host, the watch asks again.
                Err(_) => std::thread::sleep(Duration::from_secs(1)),
            }
        }
    }

    fn round(&self, addr: SocketAddr, id: &MountId, token: &str) -> io::Result<Round> {
        if !addr.ip().is_loopback() {
            return Err(io::Error::other("Laufwerk-IPC-Adresse ist nicht lokal"));
        }
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
        set_stream_timeout(&stream, Some(Duration::from_millis(MAX_WATCH_MS + 10_000)));
        let request = IpcRequest::MountHostExt {
            id: id.clone(),
            session_token: token.to_string(),
            req: MountHostRequest::PrepareLock {
                deadline_ms: WATCH_MS,
            },
        };
        write_request(&mut stream, &request)?;
        let (deadline_ms, keep_unsaved) = match read_response(&mut stream)? {
            IpcResponse::MountHostExt(MountHostReply::Idle) => return Ok(Round::Again),
            IpcResponse::MountHostExt(MountHostReply::PrepareLock {
                deadline_ms,
                keep_unsaved,
            }) => (deadline_ms, keep_unsaved),
            IpcResponse::Err { msg, .. } => return Err(io::Error::other(msg)),
            _ => return Err(io::Error::other("Unerwartete Antwort auf PrepareLock")),
        };
        let left = Duration::from_millis(deadline_ms);
        let (send, receive) = mpsc::channel();
        {
            let mut slot = self.slot();
            slot.order = Some(LockOrder {
                deadline: Instant::now() + left,
                keep_unsaved,
            });
            slot.reply = Some(send);
        }
        // Past the deadline the worker aborts the drive anyway.
        let answer = receive
            .recv_timeout(left + Duration::from_secs(1))
            .map_err(|_| io::Error::other("keine Antwort auf die Sperre"))?;
        let prepared = LockPrepared {
            pending: answer.pending as u64,
            unmounted: answer.unmounted,
            detail: answer.detail,
        };
        let mut line = serde_json::to_string(&prepared).map_err(io::Error::other)?;
        line.push('\n');
        io::Write::write_all(&mut stream, line.as_bytes())?;
        Ok(if prepared.unmounted {
            Round::Unmounted
        } else {
            Round::Again
        })
    }

    /// The order the control loop takes next.
    pub(in crate::daemon) fn take_order(&self) -> Option<LockOrder> {
        self.slot().order.take()
    }

    /// The control loop's answer to the order it took.
    pub(in crate::daemon) fn answer(&self, answer: LockAnswer) {
        if let Some(reply) = self.slot().reply.take() {
            let _ = reply.send(answer);
        }
    }
}

#[cfg(test)]
#[path = "mount_lock_participant_tests.rs"]
mod tests;
