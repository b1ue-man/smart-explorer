//! Admission policy and shared ownership of stable session promises.
//! Facts are supplied by platform adapters. These promises are accounting,
//! not physical reservations against unrelated processes or future pressure.
use super::frame_budget::{MAX_SESSION_WORKING_BYTES, MAX_WORKING_BYTES};
use std::io;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Copy, Debug)]
pub(crate) struct MemoryFacts {
    pub available_physical: u64,
    pub available_commit: u64,
    pub available_address_space: u64,
}
impl MemoryFacts {
    /// Explicit resource policy: keep three quarters for application baseline,
    /// codecs, drivers and competition; all platform constraints must be known.
    pub(crate) fn screen_capacity(self) -> u64 {
        self.available_physical
            .min(self.available_commit)
            .min(self.available_address_space)
            / 4
    }
}
#[derive(Default)]
struct State {
    capacity: u64,
    committed: u64,
    owners: usize,
    observed_ceiling: Option<u64>,
}
#[derive(Default)]
pub(crate) struct MemoryPool {
    state: Mutex<State>,
}
pub(crate) struct MemoryLease {
    pool: Arc<MemoryPool>,
    bytes: u64,
}
pub(crate) type MemoryHold = Option<Arc<MemoryLease>>;

/// Supplied platform policy, independent of the first requested allocation.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CapacityCeiling(u64);
impl CapacityCeiling {
    pub(crate) fn desktop(facts: Option<MemoryFacts>) -> Self {
        Self(if facts.is_some() {
            u64::MAX
        } else {
            MAX_WORKING_BYTES
        })
    }
    pub(crate) fn bounded(bytes: u64) -> Self {
        Self(bytes.min(MAX_WORKING_BYTES))
    }
    pub(crate) fn bytes(self) -> u64 {
        self.0
    }
}
impl MemoryPool {
    pub(crate) fn reserve(
        self: &Arc<Self>,
        facts: Option<MemoryFacts>,
        requested: u64,
    ) -> io::Result<MemoryLease> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("screen memory accounting poisoned"))?;
        // A smaller existing heap offer also bounds aggregate unknown-platform
        // admission; it must never be multiplied by the number of sessions.
        let observed = facts
            .map_or(
                MAX_WORKING_BYTES.min(requested),
                MemoryFacts::screen_capacity,
            )
            .min(state.observed_ceiling.unwrap_or(u64::MAX));
        // Freeze the aggregate envelope for an active cohort. Existing images
        // never shrink their own promises; new sessions also respect fresh facts.
        if state.owners == 0 {
            state.capacity = observed;
        }
        let ceiling = if facts.is_some() {
            MAX_SESSION_WORKING_BYTES
        } else {
            MAX_WORKING_BYTES
        };
        let bytes = requested
            .min(ceiling)
            .min(observed.saturating_sub(state.committed))
            .min(state.capacity.saturating_sub(state.committed));
        if bytes < super::frame_budget::WORKING_IMAGES * 4 {
            return Err(io::Error::other("insufficient screen session memory"));
        }
        let owners = state
            .owners
            .checked_add(1)
            .ok_or_else(|| io::Error::other("screen memory owner count overflow"))?;
        state.committed += bytes;
        state.owners = owners;
        Ok(MemoryLease {
            pool: self.clone(),
            bytes,
        })
    }

    /// A process observation narrows every existing access path. An active
    /// cohort may never grow; a fresh observation can reset an empty cohort.
    pub(crate) fn observe_capacity_ceiling(&self, ceiling: CapacityCeiling) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("screen memory accounting poisoned"))?;
        let bytes = if state.owners == 0 {
            ceiling.0
        } else {
            ceiling.0.min(state.observed_ceiling.unwrap_or(u64::MAX))
        };
        state.observed_ceiling = Some(bytes);
        if state.owners != 0 {
            state.capacity = state.capacity.min(bytes);
        }
        Ok(())
    }

    /// Exact local claims share W/X's root; no session minimum or partial grant.
    pub(super) fn reserve_exact(
        self: &Arc<Self>,
        facts: Option<MemoryFacts>,
        ceiling: CapacityCeiling,
        bytes: u64,
    ) -> io::Result<MemoryLease> {
        let maximum = facts
            .map_or(MAX_WORKING_BYTES, MemoryFacts::screen_capacity)
            .min(ceiling.0);
        if bytes == 0 || bytes > maximum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "image claim exceeds its platform policy",
            ));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("screen memory accounting poisoned"))?;
        let observed = maximum.min(state.observed_ceiling.unwrap_or(u64::MAX));
        let capacity = if state.owners == 0 {
            observed
        } else {
            state.capacity.min(observed)
        };
        if bytes > capacity.saturating_sub(state.committed) {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "image claim exceeds available process memory",
            ));
        }
        let owners = state
            .owners
            .checked_add(1)
            .ok_or_else(|| io::Error::other("screen memory owner count overflow"))?;
        if state.owners == 0 {
            state.capacity = capacity;
        }
        state.committed += bytes;
        state.owners = owners;
        Ok(MemoryLease {
            pool: self.clone(),
            bytes,
        })
    }
    #[cfg(test)]
    pub(super) fn committed_bytes(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .committed
    }

    pub(crate) fn admit_session(
        self: &Arc<Self>,
        facts: Option<MemoryFacts>,
        ceiling: CapacityCeiling,
        requested: u64,
        session: &str,
    ) -> io::Result<Arc<super::capture_admission::SessionAdmission>> {
        super::capture_admission::validate_identity(session)?;
        let maximum_total = facts
            .map_or(MAX_WORKING_BYTES, MemoryFacts::screen_capacity)
            .min(ceiling.0);
        let lease = self.reserve_with_ceiling(facts, ceiling, requested)?;
        Ok(Arc::new(super::capture_admission::SessionAdmission::new(
            session,
            lease,
            maximum_total,
        )))
    }

    /// Partial W admission remains negotiable; capture extensions are exact.
    pub(crate) fn reserve_with_ceiling(
        self: &Arc<Self>,
        facts: Option<MemoryFacts>,
        ceiling: CapacityCeiling,
        requested: u64,
    ) -> io::Result<MemoryLease> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("screen memory accounting poisoned"))?;
        let observed = facts
            .map_or(MAX_WORKING_BYTES, MemoryFacts::screen_capacity)
            .min(ceiling.0)
            .min(state.observed_ceiling.unwrap_or(u64::MAX));
        let session_limit = if facts.is_some() {
            MAX_SESSION_WORKING_BYTES
        } else {
            MAX_WORKING_BYTES
        };
        let capacity = if state.owners == 0 {
            observed
        } else {
            state.capacity
        };
        let bytes = requested
            .min(session_limit)
            .min(observed.saturating_sub(state.committed))
            .min(capacity.saturating_sub(state.committed));
        if bytes < super::frame_budget::WORKING_IMAGES * 4 {
            return Err(io::Error::other("insufficient screen session memory"));
        }
        if state.owners == 0 {
            state.capacity = capacity;
        }
        let owners = state
            .owners
            .checked_add(1)
            .ok_or_else(|| io::Error::other("screen memory owner count overflow"))?;
        state.committed += bytes;
        state.owners = owners;
        Ok(MemoryLease {
            pool: self.clone(),
            bytes,
        })
    }
}
impl MemoryLease {
    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }

    /// A session's one adjustable X lease shares exactly the same cohort as W.
    pub(super) fn empty_sibling(&self) -> Self {
        Self {
            pool: self.pool.clone(),
            bytes: 0,
        }
    }
    pub(super) fn resize_exact(&mut self, bytes: u64) -> io::Result<()> {
        let mut state = self
            .pool
            .state
            .lock()
            .map_err(|_| io::Error::other("screen memory accounting poisoned"))?;
        if bytes > self.bytes {
            let increase = bytes - self.bytes;
            let capacity = state
                .capacity
                .min(state.observed_ceiling.unwrap_or(u64::MAX));
            if increase > capacity.saturating_sub(state.committed) {
                return Err(io::Error::other("insufficient exact capture memory"));
            }
            let owners = if self.bytes == 0 {
                state
                    .owners
                    .checked_add(1)
                    .ok_or_else(|| io::Error::other("screen memory owner count overflow"))?
            } else {
                state.owners
            };
            state.committed += increase;
            state.owners = owners;
        } else {
            state.committed -= self.bytes - bytes;
            if bytes == 0 && self.bytes != 0 {
                state.owners -= 1;
            }
        }
        self.bytes = bytes;
        Ok(())
    }
    pub(super) fn release_to(&mut self, bytes: u64) {
        if bytes >= self.bytes {
            return;
        }
        let mut state = self
            .pool
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.committed = state.committed.saturating_sub(self.bytes - bytes);
        if bytes == 0 {
            state.owners = state.owners.saturating_sub(1);
        }
        self.bytes = bytes;
    }
}
impl Drop for MemoryLease {
    fn drop(&mut self) {
        if self.bytes == 0 {
            return;
        }
        let mut state = self
            .pool
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.committed = state.committed.saturating_sub(self.bytes);
        state.owners = state.owners.saturating_sub(1);
    }
}
pub(crate) fn reserve(facts: Option<MemoryFacts>, requested: u64) -> io::Result<MemoryLease> {
    pool().reserve(facts, requested)
}
pub(crate) fn reserve_with_ceiling(
    facts: Option<MemoryFacts>,
    ceiling: CapacityCeiling,
    requested: u64,
) -> io::Result<MemoryLease> {
    pool().reserve_with_ceiling(facts, ceiling, requested)
}
pub(super) fn pool() -> &'static Arc<MemoryPool> {
    static POOL: OnceLock<Arc<MemoryPool>> = OnceLock::new();
    POOL.get_or_init(|| Arc::new(MemoryPool::default()))
}
pub(crate) fn admit_session(
    facts: Option<MemoryFacts>,
    ceiling: CapacityCeiling,
    requested: u64,
    session: &str,
) -> io::Result<Arc<super::capture_admission::SessionAdmission>> {
    pool().admit_session(facts, ceiling, requested, session)
}
#[cfg(test)]
#[path = "frame_memory_tests.rs"]
mod tests;

/// Install a Java/native process envelope before opening any local image path.
pub(crate) fn observe_capacity_ceiling(ceiling: CapacityCeiling) -> io::Result<()> {
    pool().observe_capacity_ceiling(ceiling)
}
