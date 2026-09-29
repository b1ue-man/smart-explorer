//! Host admission of transfers (reads, writes, server copies, batches): the
//! buffer memory, a slot of the principal and a slot of the host are taken
//! before any work (K4). A client that declared transfer v1 is told `Busy`
//! at once when one of them is exhausted, so its adaptive flow backs off
//! instead of queueing silently behind its own deadline. Older clients wait
//! as before, but never longer than they wait themselves.
use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::share::core::eio;
use crate::share::keepalive::TRANSFER_STREAMS_PER_CONNECTION;
use crate::share::server_transfer::STREAM_BUFFER_CHUNKS;
use crate::share::session::PeerPrincipal;
use crate::transfer::MemoryReservation;

/// Transfers across all connections: half of the 512 blocking threads Tokio
/// gives the Iroh runtime (it keeps Tokio's default), so a transfer never
/// waits in Tokio's hidden spawn queue; the other half stays with the
/// 32-slot control pool, Exec, Direct repair and Iroh itself.
pub(super) const HOST_TRANSFER_SLOTS: usize = 256;

/// Transfers of one principal over all its connections: the admission it is
/// told (60). A client keeping to it (its flow plus the foreground reads it
/// keeps free) never meets this bound; one that does not cannot take the
/// slots of the others.
pub(super) const PRINCIPAL_TRANSFER_SLOTS: usize = TRANSFER_STREAMS_PER_CONNECTION as usize;

/// Buffered bytes of one transfer: the chunks in its channel, the one its
/// worker reads or writes and the one its stream sends or receives (data
/// frames never carry more than one chunk).
const TRANSFER_BUFFER_BYTES: u64 = ((STREAM_BUFFER_CHUNKS + 2) * crate::share::fs::CHUNK) as u64;

/// Poll interval of an older client's wait for buffer memory; the memory
/// budget waits in the same 100 ms slices.
const MEMORY_WAIT: Duration = Duration::from_millis(100);

/// Longest an older client's transfer waits for memory and slots. Such a
/// client gives up 60 s after sending its request; stopping 15 s earlier
/// leaves the reply (open and `Ready`, at most a round trip) time to arrive
/// first, so no target is created for a client that already left.
pub(super) const LEGACY_WAIT: Duration = Duration::from_secs(45);
const _: () = assert!(LEGACY_WAIT.as_secs() < crate::share::io_deadline::PEER_OP_TIMEOUT.as_secs());

fn host_slots() -> Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS
        .get_or_init(|| Arc::new(Semaphore::new(HOST_TRANSFER_SLOTS)))
        .clone()
}

type PrincipalSlots = Mutex<HashMap<PeerPrincipal, Weak<Semaphore>>>;

fn principal_table() -> &'static PrincipalSlots {
    static TABLE: OnceLock<PrincipalSlots> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The slots of `principal`; they live while one of its transfers runs.
fn principal_slots(principal: &PeerPrincipal) -> Arc<Semaphore> {
    let mut table = principal_table()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(slots) = table.get(principal).and_then(Weak::upgrade) {
        return slots;
    }
    // A new semaphore is created only when none of the principal's
    // transfers runs; entries of idle principals are dropped then.
    table.retain(|_, slots| slots.strong_count() > 0);
    let slots = Arc::new(Semaphore::new(PRINCIPAL_TRANSFER_SLOTS));
    table.insert(principal.clone(), Arc::downgrade(&slots));
    slots
}

/// Transfers `principal` runs right now (tests observe slot releases).
#[cfg(test)]
pub(super) fn principal_in_use(principal: &PeerPrincipal) -> usize {
    let table = principal_table()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    table
        .get(principal)
        .and_then(Weak::upgrade)
        .map_or(0, |slots| {
            PRINCIPAL_TRANSFER_SLOTS.saturating_sub(slots.available_permits())
        })
}

/// Held for the whole transfer; dropping it returns memory and both slots.
pub(super) struct TransferSlot {
    _memory: MemoryReservation,
    _principal: OwnedSemaphorePermit,
    _host: OwnedSemaphorePermit,
}

/// Admits at once or answers `Busy` (clients that declared transfer v1).
pub(super) fn admit_now(principal: &PeerPrincipal) -> io::Result<TransferSlot> {
    try_admit(host_slots(), principal_slots(principal))
}

/// Waits for memory and slots (older clients); the caller bounds the wait
/// with [`wait_while_present`].
pub(super) async fn admit_waiting(principal: &PeerPrincipal) -> io::Result<TransferSlot> {
    wait_admit(host_slots(), principal_slots(principal)).await
}

/// Waits for `admission` at most `limit` and never beyond the moment `gone`
/// completes (the client stopped the stream or the connection ended).
pub(super) async fn wait_while_present<T>(
    admission: impl Future<Output = io::Result<T>>,
    gone: impl Future<Output = ()>,
    limit: Duration,
) -> io::Result<T> {
    tokio::select! {
        admitted = tokio::time::timeout(limit, admission) => match admitted {
            Ok(admitted) => admitted,
            Err(_) => Err(busy("keine Übertragung wurde rechtzeitig frei")),
        },
        () = gone => Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "Client hat die Anfrage vor ihrer Aufnahme aufgegeben",
        )),
    }
}

/// Memory first, then the slots (K2 order); nothing waits.
fn try_admit(host: Arc<Semaphore>, principal: Arc<Semaphore>) -> io::Result<TransferSlot> {
    let memory = crate::transfer::try_reserve_memory(TRANSFER_BUFFER_BYTES)
        .ok_or_else(|| busy("der Arbeitsspeicher für Übertragungen ist belegt"))?;
    let principal = principal
        .try_acquire_owned()
        .map_err(|_| busy("dieses Gerät nutzt alle ihm zugesagten Übertragungen"))?;
    let host = host
        .try_acquire_owned()
        .map_err(|_| busy("alle Übertragungsplätze des Hosts sind belegt"))?;
    Ok(TransferSlot {
        _memory: memory,
        _principal: principal,
        _host: host,
    })
}

async fn wait_admit(host: Arc<Semaphore>, principal: Arc<Semaphore>) -> io::Result<TransferSlot> {
    let memory = wait_for_memory().await;
    let principal = principal.acquire_owned().await.map_err(|_| closed())?;
    let host = host.acquire_owned().await.map_err(|_| closed())?;
    Ok(TransferSlot {
        _memory: memory,
        _principal: principal,
        _host: host,
    })
}

pub(super) fn busy(reason: &str) -> io::Error {
    crate::vfs::congestion_error(format!("Share-Host ausgelastet: {reason}"), None)
}

fn closed() -> io::Error {
    eio("Share-Übertragungsaufnahme ist geschlossen")
}

async fn wait_for_memory() -> MemoryReservation {
    loop {
        if let Some(memory) = crate::transfer::try_reserve_memory(TRANSFER_BUFFER_BYTES) {
            return memory;
        }
        tokio::time::sleep(MEMORY_WAIT).await;
    }
}

#[cfg(test)]
#[path = "server_admission_tests.rs"]
mod tests;
