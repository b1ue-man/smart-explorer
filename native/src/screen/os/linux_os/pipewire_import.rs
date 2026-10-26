//! One admitted whole mapping per Core memblock. Native access is serialized
//! by the thread loop; no callback waits for Parent admission.
use super::super::pipewire_abi::memory::{
    fd_facts, CoreEvents, FdFacts, MemBlock, MemMap, MemoryApi, MempoolEvents,
};
use super::super::pipewire_abi::{CoreMethods, SpaInterface};
use super::*;
use crate::screen::core::capture_admission::CaptureSlice;
use crate::screen::core::capture_import::{
    CaptureImportConnection, CaptureImportGrant, CaptureImportId, CaptureImportNeed,
};
use std::sync::atomic::{AtomicBool, AtomicU64};

#[derive(Clone, Copy)]
pub(super) struct Pending {
    pub mem_id: u32,
    pub kind: u32,
    pub fd: c_int,
    pub flags: u32,
    pub id: CaptureImportId,
}
#[derive(Clone, Copy)]
pub(super) struct Inspected {
    pub pending: Pending,
    pub block: usize,
    pub facts: FdFacts,
    pub rights: u32,
}
/// Its fixed allocation is claimed as Metadata before Box allocation.
/// remove_mem alone does not end this node or its mapping promise.
pub(super) struct ImportBacking {
    pub next: Option<Box<ImportBacking>>,
    pub mem_id: u32,
    pub id: CaptureImportId,
    pub block: usize,
    pub fd: c_int,
    pub facts: FdFacts,
    pub base: usize,
    pub rights: u32,
    pub map: Option<usize>,
    pub retiring: bool,
    pub removed: bool,
    pub grant: CaptureImportGrant,
    pub mapping_slice: Option<CaptureSlice>,
    pub metadata_slice: Option<CaptureSlice>,
}
impl ImportBacking {
    fn retire(mut owner: Box<Self>) {
        let mapping = owner.mapping_slice.take();
        let metadata = owner.metadata_slice.take();
        let grant = owner.grant.control_grant();
        // Deallocate the table node before releasing its Metadata promise.
        drop(owner);
        drop(mapping);
        drop(metadata);
        let _ = grant.retire();
    }
}
#[derive(Default)]
pub(super) struct Imports {
    pub pending: Option<Pending>,
    pub head: Option<Box<ImportBacking>>,
    pub pool_closing: bool,
}
pub(super) struct ImportController {
    pub api: MemoryApi,
    pub latest: Arc<LatestFrame>,
    pub memory: Arc<CaptureMemory>,
    pub core: AtomicPtr<c_void>,
    pub pool: AtomicPtr<c_void>,
    pub state: Mutex<Imports>,
    pub lifecycle: Mutex<()>,
    pub closed: AtomicBool,
    pub sequence: AtomicU64,
    pub generation: u64,
    pub(super) connection: Mutex<Option<CaptureImportConnection>>,
    pub(super) core_hook: Box<SpaHook>,
    pub(super) pool_hook: Box<SpaHook>,
}
// SAFETY: native pointers and hooks are touched only under the loop lock.
unsafe impl Send for ImportController {}
unsafe impl Sync for ImportController {}
impl ImportController {
    pub(super) fn next_id(&self) -> io::Result<CaptureImportId> {
        let previous = self
            .sequence
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map_err(|_| io::Error::other("pipewire import sequence exhausted"))?;
        CaptureImportId::new(self.generation, previous + 1)
    }
    pub(super) fn fail(&self, error: impl ToString) {
        self.latest.block_import(true);
        self.latest.fail(error.to_string());
        self.memory.signal_stop();
    }
    pub(super) fn close(&self) {
        // Resume may dispatch callbacks, but those never take lifecycle.
        let _lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        {
            let _state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            self.closed.store(true, Ordering::Release);
        }
        self.latest.block_import(true);
        self.connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.memory.wake();
    }
    pub(super) fn open(&self) -> bool {
        !self.closed.load(Ordering::Acquire) && !self.memory.stopped()
    }
    pub(super) fn pending(&self) -> Option<Pending> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pending
    }
    pub(super) unsafe fn inspect(&self, pending: Pending) -> io::Result<Inspected> {
        if !self.open() || self.core.load(Ordering::Acquire).is_null() {
            return Err(io::Error::other("pipewire import closed"));
        }
        let block = (self.api.find_id)(self.pool.load(Ordering::Acquire), pending.mem_id);
        let native = block
            .as_ref()
            .ok_or_else(|| io::Error::other("pipewire block missing"))?;
        if native.fd != pending.fd || native.type_ != pending.kind || native.flags != pending.flags
        {
            return Err(io::Error::other("pipewire import identity changed"));
        }
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut node = state.head.as_ref();
        while let Some(owner) = node {
            if owner.block == block as usize && !owner.removed {
                return Err(io::Error::other("pipewire repeated a mapped Core object"));
            }
            node = owner.next.as_ref();
        }
        drop(state);
        let facts = fd_facts(pending.fd)?;
        let rights = pending.flags & 3;
        if rights == 0
            || rights & 2 != 0 && facts.access == libc::O_RDONLY
            || rights & 1 != 0 && facts.access == libc::O_WRONLY
        {
            return Err(io::Error::other("pipewire mapping rights differ from FD"));
        }
        Ok(Inspected {
            pending,
            block: block as usize,
            facts,
            rights,
        })
    }
    pub(super) fn need(&self, inspected: Inspected) -> io::Result<CaptureImportNeed> {
        CaptureImportNeed::new(
            self.memory.source().clone(),
            inspected.pending.id,
            inspected.facts.mapped,
            std::mem::size_of::<ImportBacking>() as u64,
        )
    }
    pub(super) unsafe fn map_approved(
        &self,
        inspected: Inspected,
        grant: CaptureImportGrant,
    ) -> io::Result<()> {
        // No Parent wait is inside this gate. Close waits for an in-progress
        // native resume to return before declaring this connection terminal.
        let _lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        grant.check_identity(self.memory.source(), &inspected.pending.id)?;
        let current = self.inspect(inspected.pending)?;
        if current.block != inspected.block
            || current.rights != inspected.rights
            || current.facts.dev != inspected.facts.dev
            || current.facts.inode != inspected.facts.inode
            || current.facts.access != inspected.facts.access
        {
            return Err(io::Error::other(
                "pipewire FD identity changed during admission",
            ));
        }
        if current.facts != inspected.facts {
            // A changed length needs a new exact request and monotone ID.
            // Keep Core paused; this response grants no mapping of the new size.
            grant.retire()?;
            let mut pending = current.pending;
            pending.id = self.next_id()?;
            self.state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pending = Some(pending);
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "pipewire import length changed; readmit",
            ));
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if !self.open() || state.pending.map(|p| p.id) != Some(inspected.pending.id) {
            return Err(io::Error::other("pipewire stale or closed import response"));
        }
        // memblock_map dispatches no protocol callbacks. The Core reference
        // keeps this block alive during the short serialized claim/map.
        let mapping_slice = grant.claim_mapping()?;
        let metadata_slice = grant.claim_metadata(std::mem::size_of::<ImportBacking>() as u64)?;
        let mut owner = Box::new(ImportBacking {
            next: None,
            mem_id: inspected.pending.mem_id,
            id: inspected.pending.id,
            block: inspected.block,
            fd: inspected.pending.fd,
            facts: inspected.facts,
            base: 0,
            rights: inspected.rights,
            map: None,
            retiring: false,
            removed: false,
            grant,
            mapping_slice: Some(mapping_slice),
            metadata_slice: Some(metadata_slice),
        });
        let map = (self.api.map)(
            inspected.block as *mut MemBlock,
            inspected.rights,
            0,
            inspected.facts.length,
            null_mut(),
        );
        if map.is_null() {
            ImportBacking::retire(owner);
            return Err(io::Error::other("pipewire whole-object mapping failed"));
        }
        owner.base = (*map).ptr as usize;
        owner.map = Some(map as usize);
        let valid = (*map).block as usize == inspected.block
            && (*map).offset == 0
            && (*map).size == inspected.facts.length
            && owner.base != 0;
        owner.next = state.head.take();
        state.head = Some(owner);
        state.pending = None;
        drop(state);
        if !valid {
            return Err(io::Error::other(
                "pipewire whole map returned a different range",
            ));
        }
        // Reentrant add_mem may set the barrier again during this resume.
        self.latest.block_import(false);
        let result = (self.api.set_paused)(self.core.load(Ordering::Acquire), false);
        if result < 0 {
            return Err(io::Error::other(format!(
                "pipewire core resume failed: {result}"
            )));
        }
        Ok(())
    }
    /// Only after callback/resume returned, at an exclusive-loop control point.
    pub(super) unsafe fn reap(&self, core_finished: bool) {
        loop {
            let map = {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                let mut item = state.head.as_mut();
                let mut map = None;
                while let Some(owner) = item {
                    if !owner.removed && (owner.retiring || core_finished) {
                        map = owner.map.take();
                        if map.is_some() {
                            break;
                        }
                    }
                    item = owner.next.as_mut();
                }
                map
            };
            let Some(map) = map else { break };
            // free_map may call removed; state is deliberately unlocked.
            (self.api.free_map)(map as *mut MemMap);
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let closing = state.pool_closing;
        let mut pending = state.head.take();
        let mut retained = None;
        while let Some(mut owner) = pending {
            pending = owner.next.take();
            if owner.removed || core_finished || closing {
                ImportBacking::retire(owner);
            } else {
                owner.next = retained.take();
                retained = Some(owner);
            }
        }
        state.head = retained;
    }
    pub(super) unsafe fn reap_release_handles(&self) {
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let mut node = state.head.as_mut();
            while let Some(owner) = node {
                owner.retiring = true;
                node = owner.next.as_mut();
            }
        }
        self.reap(false);
    }
    pub(super) unsafe fn finish_core(&self) {
        // core_disconnect returned, including forced DONT_NOTIFY destruction.
        // Every memmap handle was destroyed by Core and must not be reused.
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let mut node = state.head.as_mut();
            while let Some(owner) = node {
                owner.map = None;
                node = owner.next.as_mut();
            }
        }
        self.reap(true);
        self.core.store(null_mut(), Ordering::Release);
        self.pool.store(null_mut(), Ordering::Release);
    }
}
#[path = "pipewire_import_control.rs"]
pub(super) mod control;
#[path = "pipewire_import_events.rs"]
mod events;
#[cfg(test)]
pub(super) use events::fixture_support;
