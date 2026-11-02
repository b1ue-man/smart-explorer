//! Resource reservations for retained results and queued job metadata.
use std::{
    io,
    sync::atomic::{AtomicU64, Ordering},
};
static DISK: AtomicU64 = AtomicU64::new(0);
static META: AtomicU64 = AtomicU64::new(0);
#[derive(Default)]
pub(super) struct Reservation {
    bytes: u64,
    disk: bool,
}
impl Reservation {
    pub(super) fn metadata(bytes: u64) -> io::Result<Self> {
        let mut hold = Self {
            bytes: 0,
            disk: false,
        };
        hold.add(bytes)?;
        Ok(hold)
    }
    pub(super) fn disk() -> Self {
        Self {
            bytes: 0,
            disk: true,
        }
    }
    pub(super) fn add(&mut self, bytes: u64) -> io::Result<()> {
        let memory = crate::transfer::memory_budget() as u64;
        let limit = if self.disk {
            memory.saturating_mul(4)
        } else {
            memory / 16
        };
        let counter = if self.disk { &DISK } else { &META };
        counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(bytes).filter(|used| *used <= limit)
            })
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::OutOfMemory,
                    "Speicherbudget für vorgehaltene Analyseaufträge ausgeschöpft",
                )
            })?;
        self.bytes = self
            .bytes
            .checked_add(bytes)
            .ok_or_else(|| io::Error::other("Analyse-Reservierung übergelaufen"))?;
        Ok(())
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        (if self.disk { &DISK } else { &META }).fetch_sub(self.bytes, Ordering::Relaxed);
    }
}
