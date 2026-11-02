//! OS-free admission using caller-available space supplied by the host.
use super::engine::{lock, MountEngine};
use std::{
    io,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

pub trait CacheSpaceProbe: Send + Sync {
    fn available_bytes(&self) -> io::Result<u64>;
}

pub(super) const CACHE_RESERVE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Default)]
pub(super) struct CacheSpace {
    probe: Option<Arc<dyn CacheSpaceProbe>>,
    pub(super) capacity: Arc<super::capacity_refresh::CapacityCache>,
    admission: Mutex<()>,
    pending: AtomicU64,
}

pub(super) struct GrowthReservation {
    space: Arc<CacheSpace>,
    bytes: u64,
}

impl Drop for GrowthReservation {
    fn drop(&mut self) {
        self.space.pending.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

impl MountEngine {
    pub fn with_cache_space_probe(mut self, probe: Arc<dyn CacheSpaceProbe>) -> Self {
        self.cache_space = Arc::new(CacheSpace {
            probe: Some(probe),
            capacity: Arc::default(),
            admission: Mutex::new(()),
            pending: AtomicU64::new(0),
        });
        super::capacity_refresh::start(
            Arc::clone(&self.cache_space.capacity),
            Arc::clone(&self.backend),
            self.projector.root().as_str().to_string(),
        );
        self
    }

    pub fn with_capacity_observer(self, observer: Arc<dyn Fn() + Send + Sync>) -> Self {
        if let Ok(mut slot) = self.cache_space.capacity.observer.lock() {
            *slot = Some(observer);
        }
        self
    }

    /// No backend or network I/O: suitable for native filesystem callbacks.
    pub fn capacity_snapshot(&self) -> crate::vfs::CapacitySnapshot {
        self.cache_space
            .capacity
            .value
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
            .unwrap_or_else(|| {
                crate::vfs::CapacitySnapshot::unknown(
                    self.backend.state_identity(),
                    self.projector.root().as_str(),
                )
            })
    }

    pub fn cache_capacity_budget(&self, available_bytes: u64) -> u64 {
        available_bytes
            .saturating_sub(CACHE_RESERVE_BYTES)
            .saturating_sub(self.cache_space.pending.load(Ordering::Acquire))
    }

    pub(super) fn reserve_growth(&self, bytes: u64) -> io::Result<GrowthReservation> {
        if bytes == 0 {
            return Ok(GrowthReservation {
                space: Arc::clone(&self.cache_space),
                bytes: 0,
            });
        }
        self.reserve_space(bytes)
    }

    fn reserve_space(&self, bytes: u64) -> io::Result<GrowthReservation> {
        let space = &self.cache_space;
        let _admission = lock(&space.admission)?;
        let pending = space.pending.load(Ordering::Acquire);
        let total = pending
            .checked_add(bytes)
            .ok_or_else(|| io::Error::other("cache growth overflow"))?;
        if let Some(probe) = &space.probe {
            let required = total
                .checked_add(CACHE_RESERVE_BYTES)
                .ok_or_else(|| io::Error::other("cache reserve overflow"))?;
            while probe.available_bytes()? < required {
                if !self.clean_cache.evict_oldest(&self.spool)? {
                    return Err(io::Error::new(io::ErrorKind::StorageFull,
                        "mount working space would consume its 512 MiB safety reserve; open and dirty files were preserved"));
                }
            }
        }
        // Completed writers can release reservations while admission is locked.
        space.pending.fetch_add(bytes, Ordering::AcqRel);
        Ok(GrowthReservation {
            space: Arc::clone(space),
            bytes,
        })
    }

    pub(super) fn maintain_space(&self) -> io::Result<()> {
        let _reservation = self.reserve_space(0)?;
        Ok(())
    }
}

impl Drop for CacheSpace {
    fn drop(&mut self) {
        self.capacity.stop();
    }
}
