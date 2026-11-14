//! Process budget of the copy buffers of TCP pumps (plan 2026-11-14, E7 b and C9).
//!
//! Each pump direction holds a base buffer of `TCP_COPY_BYTES` while it runs.
//! A base buffer grows once to `TCP_COPY_BYTES_GROWN` when a read fills it, and
//! only if the total of all held copy buffers stays within
//! `TCP_COPY_BUDGET_BYTES`. A direction returns its base buffer and its growth
//! when it ends, so the held total returns to its previous value.
use crate::share::tcp_forward::{TCP_COPY_BUDGET_BYTES, TCP_COPY_BYTES, TCP_COPY_BYTES_GROWN};
use std::sync::atomic::{AtomicU64, Ordering};

/// Bytes of copy buffers held by the running pump directions of one process.
pub(crate) struct CopyBudget {
    held: AtomicU64,
    capacity: u64,
}

impl CopyBudget {
    pub(crate) const fn new(capacity: u64) -> Self {
        Self {
            held: AtomicU64::new(0),
            capacity,
        }
    }

    /// Holds base bytes. Not refused: base buffers keep the existing bounded
    /// contract of `tcp_forward` (tunnel and stream caps); only growth is
    /// limited by the capacity.
    pub(crate) fn hold(&self, bytes: u64) {
        self.held.fetch_add(bytes, Ordering::AcqRel);
    }

    /// Holds growth bytes only if the total stays within the capacity.
    pub(crate) fn try_grow(&self, bytes: u64) -> bool {
        self.held
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                held.checked_add(bytes)
                    .filter(|next| *next <= self.capacity)
            })
            .is_ok()
    }

    /// Returns held bytes. Each direction returns exactly what it held, so the
    /// total never reaches the lower bound; the bound keeps the counter valid
    /// even if an accounting pair were ever broken.
    pub(crate) fn give_back(&self, bytes: u64) {
        let _ = self
            .held
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                Some(held.saturating_sub(bytes))
            });
    }

    /// Bytes held now.
    #[cfg_attr(not(test), allow(dead_code))] // diagnostics and tests
    pub(crate) fn held(&self) -> u64 {
        self.held.load(Ordering::Acquire)
    }
}

/// The budget of this process.
pub(crate) fn process_budget() -> &'static CopyBudget {
    static PROCESS_BUDGET: CopyBudget = CopyBudget::new(TCP_COPY_BUDGET_BYTES);
    &PROCESS_BUDGET
}

#[cfg(any(test, feature = "test-support"))]
static LEGACY_COPY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Test switch of measurement M4 (plan E7): `true` selects the copy mode before
/// this plan (one route wrapper per operation, fixed 8-KiB buffers).
#[cfg(any(test, feature = "test-support"))]
#[allow(dead_code)] // set by the M4 measurement harness (plan P7)
pub(crate) fn set_legacy_copy(enabled: bool) {
    LEGACY_COPY.store(enabled, Ordering::Release);
}

/// Whether new pumps use the copy mode before this plan.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn legacy_copy() -> bool {
    LEGACY_COPY.load(Ordering::Acquire)
}

/// Production builds have no switch.
#[cfg(not(any(test, feature = "test-support")))]
pub(crate) fn legacy_copy() -> bool {
    false
}

/// One copy direction's buffer. It holds its base bytes in the budget while it
/// lives and returns them, with any growth, when it drops.
pub(crate) struct CopyBuffer {
    bytes: Vec<u8>,
    budget: &'static CopyBudget,
    growable: bool,
    grown: u64,
}

impl CopyBuffer {
    pub(crate) fn new(budget: &'static CopyBudget, growable: bool) -> Self {
        budget.hold(TCP_COPY_BYTES as u64);
        Self {
            bytes: vec![0; TCP_COPY_BYTES],
            budget,
            growable,
            grown: 0,
        }
    }

    /// The whole buffer, for the next read.
    pub(crate) fn slot(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    /// The first `length` bytes after a read.
    pub(crate) fn filled(&self, length: usize) -> &[u8] {
        &self.bytes[..length.min(self.bytes.len())]
    }

    /// The current size of the buffer.
    pub(crate) fn size(&self) -> usize {
        self.bytes.len()
    }

    /// Called after a read filled the buffer: grows it once to
    /// `TCP_COPY_BYTES_GROWN` if growth is allowed and the budget has room.
    pub(crate) fn grow_if_allowed(&mut self) {
        if !self.growable || self.bytes.len() >= TCP_COPY_BYTES_GROWN {
            return;
        }
        let extra = (TCP_COPY_BYTES_GROWN - self.bytes.len()) as u64;
        if self.budget.try_grow(extra) {
            self.grown += extra;
            self.bytes.resize(TCP_COPY_BYTES_GROWN, 0);
        }
    }
}

impl Drop for CopyBuffer {
    fn drop(&mut self) {
        self.budget.give_back(TCP_COPY_BYTES as u64 + self.grown);
    }
}
