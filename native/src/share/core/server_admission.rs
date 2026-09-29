//! Host admission of transfers (reads, writes, server copies, batches): the
//! buffer memory and a slot of the host are taken before any work (K4). A
//! client that declared transfer v1 is told `Busy` at once when either is
//! exhausted, so its adaptive flow backs off instead of queueing silently
//! behind its own deadline; older clients wait as before.
//!
//! One connection never runs more transfers than it has streams (64), so the
//! host needs no bound of its own per connection: a client keeping to the
//! advertised admission (60) still has four streams for browsing, previews
//! and status queries, and those are never refused for its own transfers.
use std::io;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::share::core::eio;
use crate::share::server_transfer::STREAM_BUFFER_CHUNKS;
use crate::transfer::MemoryReservation;

/// Transfers across all connections: half of the 512 blocking threads Tokio
/// gives the Iroh runtime (it keeps Tokio's default), so a transfer never
/// waits in Tokio's hidden spawn queue; the other half stays with the
/// 32-slot control pool, Exec, Direct repair and Iroh itself.
const HOST_TRANSFER_SLOTS: usize = 256;

/// Buffered bytes of one transfer worker: its channel of chunks plus the one
/// being read or written.
const TRANSFER_BUFFER_BYTES: u64 = ((STREAM_BUFFER_CHUNKS + 1) * crate::share::fs::CHUNK) as u64;

/// Poll interval of an older client's wait for buffer memory; the memory
/// budget waits in the same 100 ms slices.
const MEMORY_WAIT: Duration = Duration::from_millis(100);

fn host_slots() -> Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS
        .get_or_init(|| Arc::new(Semaphore::new(HOST_TRANSFER_SLOTS)))
        .clone()
}

/// Held for the whole transfer; dropping it returns the memory and the slot.
pub(super) struct TransferSlot {
    _memory: MemoryReservation,
    _slot: OwnedSemaphorePermit,
}

/// Memory first, then the slot (K2 order); `fail_fast` never waits.
pub(super) async fn admit(fail_fast: bool) -> io::Result<TransferSlot> {
    admit_from(host_slots(), fail_fast).await
}

async fn admit_from(slots: Arc<Semaphore>, fail_fast: bool) -> io::Result<TransferSlot> {
    if fail_fast {
        let memory = crate::transfer::try_reserve_memory(TRANSFER_BUFFER_BYTES)
            .ok_or_else(|| busy("der Arbeitsspeicher für Übertragungen ist belegt"))?;
        let slot = slots
            .try_acquire_owned()
            .map_err(|_| busy("alle Übertragungsplätze des Hosts sind belegt"))?;
        return Ok(TransferSlot {
            _memory: memory,
            _slot: slot,
        });
    }
    let memory = wait_for_memory().await;
    let slot = slots
        .acquire_owned()
        .await
        .map_err(|_| eio("Share-Übertragungsaufnahme ist geschlossen"))?;
    Ok(TransferSlot {
        _memory: memory,
        _slot: slot,
    })
}

fn busy(reason: &str) -> io::Error {
    crate::vfs::congestion_error(format!("Share-Host ausgelastet: {reason}"), None)
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
mod tests {
    use super::*;

    #[test]
    fn transfer_engine_task_admission_answers_busy_when_full() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let slots = Arc::new(Semaphore::new(2));
            let first = admit_from(slots.clone(), true).await.expect("free slot");
            let second = admit_from(slots.clone(), true).await.expect("free slot");
            let refused = admit_from(slots.clone(), true)
                .await
                .err()
                .expect("the host is full");
            let congestion = crate::vfs::congestion_of(&refused).expect("full is congestion");
            assert!(
                congestion.message.contains("ausgelastet"),
                "{}",
                congestion.message
            );

            // An older client waits instead and proceeds once a slot frees.
            let waiting =
                tokio::time::timeout(Duration::from_millis(50), admit_from(slots.clone(), false));
            assert!(waiting.await.is_err(), "no slot may be free yet");
            drop(first);
            let admitted =
                tokio::time::timeout(Duration::from_secs(5), admit_from(slots.clone(), false))
                    .await
                    .expect("a freed slot admits the waiting transfer");
            assert!(admitted.is_ok());
            drop(second);
            assert!(admit_from(slots, true).await.is_ok());
            assert_eq!(TRANSFER_BUFFER_BYTES, 768 * 1024);
        });
    }
}
