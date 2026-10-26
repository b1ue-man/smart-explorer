//! Core receive barrier and deferred native lifetime notifications.
use super::*;
pub(super) static CORE: CoreEvents = CoreEvents {
    version: 0,
    info: None,
    done: None,
    ping: None,
    error: Some(error),
    remove_id: None,
    bound_id: None,
    add_mem: Some(add_mem),
    remove_mem: Some(remove_mem),
};
pub(super) static POOL: MempoolEvents = MempoolEvents {
    version: 0,
    destroy: Some(destroy),
    added: None,
    removed: Some(removed),
};
fn callback(pointer: *mut c_void, work: impl FnOnce(&ImportController)) {
    // SAFETY: the stable box outlives both listeners and native destruction.
    let data = unsafe { &*(pointer as *const ImportController) };
    if catch_unwind(AssertUnwindSafe(|| work(data))).is_err() {
        data.fail("pipewire import callback panic");
    }
}
unsafe extern "C" fn add_mem(
    pointer: *mut c_void, mem_id: u32, kind: u32, fd: c_int, flags: u32,
) {
    callback(pointer, |data| {
        data.latest.block_import(true);
        let core = data.core.load(Ordering::Acquire);
        if core.is_null() || unsafe { (data.api.set_paused)(core, true) } < 0 {
            data.fail("pipewire core import pause failed");
            return;
        }
        if !data.open() {
            return;
        }
        let id=match data.next_id() {
            Ok(id)=>id,
            Err(error)=>{data.fail(error);return;}
        };
        let mut state = data.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.pending.is_some() {
            drop(state);
            data.fail("pipewire delivered an import across its receive barrier");
            return;
        }
        state.pending = Some(Pending { mem_id, kind, fd, flags, id });
        drop(state);
        data.memory.wake();
    });
}
unsafe extern "C" fn remove_mem(pointer: *mut c_void, id: u32) {
    callback(pointer, |data| {
        let mut state = data.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut node = state.head.as_mut();
        while let Some(owner) = node {
            if owner.mem_id == id && !owner.retiring && !owner.removed {
                owner.retiring = true;
                break;
            }
            node = owner.next.as_mut();
        }
        drop(state);
        data.memory.wake();
    });
}
unsafe extern "C" fn removed(pointer: *mut c_void, block: *mut MemBlock) {
    callback(pointer, |data| {
        let mut state = data.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut node = state.head.as_mut();
        while let Some(owner) = node {
            if owner.block == block as usize {
                // removed can run before the forced-unmap stack returned.
                // Invalidate handles now; leave both slices for control reap.
                owner.map = None;
                owner.removed = true;
            }
            node = owner.next.as_mut();
        }
        drop(state);
        data.memory.wake();
    });
}
unsafe extern "C" fn destroy(pointer: *mut c_void) {
    callback(pointer, |data| {
        let mut state = data.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.pool_closing = true;
        let mut node = state.head.as_mut();
        while let Some(owner) = node {
            owner.map = None;
            owner.removed = true;
            node = owner.next.as_mut();
        }
        drop(state);
        data.core.store(null_mut(), Ordering::Release);
        data.pool.store(null_mut(), Ordering::Release);
        data.fail("pipewire core mempool destroyed");
    });
}
unsafe extern "C" fn error(
    pointer: *mut c_void, _id: u32, _seq: c_int, result: c_int, message: *const c_char,
) {
    callback(pointer, |data| {
        let message = if message.is_null() {
            "unknown".into()
        } else {
            unsafe { CStr::from_ptr(message) }.to_string_lossy()
        };
        data.fail(format!("pipewire core error {result}: {message}"));
    });
}
impl ImportController {
    pub(in crate::screen::os::pipewire_stream) fn new(
        api: MemoryApi, latest: Arc<LatestFrame>, memory: Arc<CaptureMemory>,
    ) -> io::Result<Box<Self>> {
        let connection = memory.import_connection()?;
        Ok(Box::new(Self {
            api,
            latest,
            memory,
            core: AtomicPtr::new(null_mut()),
            pool: AtomicPtr::new(null_mut()),
            state: Mutex::new(Imports::default()),
            lifecycle:Mutex::new(()),
            closed: AtomicBool::new(false),
            sequence: AtomicU64::new(0),
            generation: connection.generation(),
            connection: Mutex::new(Some(connection)),
            core_hook: Box::new(SpaHook::zeroed()),
            pool_hook: Box::new(SpaHook::zeroed()),
        }))
    }
    /// Called before stream_connect and before event dispatch can start.
    pub(in crate::screen::os::pipewire_stream) unsafe fn attach(&mut self, core: *mut c_void) -> io::Result<()> {
        let pool = (self.api.get_mempool)(core);
        if pool.is_null() {
            return Err(io::Error::other("pipewire core has no mempool"));
        }
        self.core.store(core, Ordering::Release);
        self.pool.store(pool, Ordering::Release);
        let interface = &*(core as *const SpaInterface);
        let methods = (interface.funcs as *const CoreMethods)
            .as_ref().ok_or_else(|| io::Error::other("pipewire core methods missing"))?;
        let listen = methods.add_listener
            .ok_or_else(|| io::Error::other("pipewire core listener missing"))?;
        let pointer = (self as *mut Self).cast();
        let result = listen(
            interface.data, &mut *self.core_hook, (&CORE as *const CoreEvents).cast(), pointer,
        );
        if result < 0 {
            return Err(io::Error::other("pipewire core import listener failed"));
        }
        (self.api.add_listener)(pool, &mut *self.pool_hook, &POOL, pointer);
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod fixture_support {
    use super::*;
    use crate::screen::os::pipewire_stream::{imports, memory::CaptureMemory, buffers::fixture};
    use crate::screen::os::pipewire_abi::{SpaInterface, CoreMethods};
    use crate::screen::core::capture_admission::{CaptureGrant, SessionAdmission, SourceKey};
    use crate::screen::core::host_codec_adapters::{HostOutputPlan, HostContract, HostCodecPlan};
    use crate::screen::core::frame_memory::{MemoryPool, MemoryFacts, CapacityCeiling};
    use crate::screen::core::frame_budget::MAX_WORKING_BYTES;
    pub(crate) struct Fixture {
        pub(crate) imports: Box<imports::ImportController>,
        pub(crate) memory: Arc<CaptureMemory>,
        pub(crate) admission: Arc<SessionAdmission>,
        pub(crate) latest: Arc<LatestFrame>,
        core: Box<SpaInterface>,
    }
    impl Fixture {
        pub(crate) fn new(session: &str) -> Self {
            fixture::reset();
            let facts = Some(MemoryFacts {
                available_physical: 8 << 30,
                available_commit: 8 << 30,
                available_address_space: 8 << 30,
            });
            let admission = Arc::new(MemoryPool::default()).admit_session(
                facts, CapacityCeiling::desktop(facts), MAX_WORKING_BYTES, session,
            ).unwrap();
            let key = SourceKey::new(session, "portal:selected-node").unwrap();
            admission.register_prepared_source(key.clone()).unwrap();
            let memory = Arc::new(CaptureMemory::new(key, admission.clone()));
            let latest = Arc::new(LatestFrame::default());
            latest.suspend_before_connect();
            let mut imports = imports::ImportController::new(
                fixture::api(), latest.clone(), memory.clone(),
            ).unwrap();
            static METHODS: CoreMethods = CoreMethods {
                version: 0,
                add_listener: Some(fixture::core_listen),
                hello: None,
                sync: None,
                pong: None,
                error: None,
                get_registry: None,
            };
            let mut core = Box::new(SpaInterface {
                type_: std::ptr::null(),
                version: 3,
                funcs: (&METHODS as *const CoreMethods).cast(),
                data: std::ptr::null_mut(),
            });
            unsafe { imports.attach((&mut *core as *mut SpaInterface).cast()).unwrap() };
            Self { imports, memory, latest, core, admission }
        }
        pub(crate) fn inspect(&self) -> imports::Inspected {
            unsafe { self.imports.inspect(self.imports.pending().unwrap()).unwrap() }
        }
        pub(crate) fn admit(&self) {
            let inspected = self.inspect();
            let need = self.imports.need(inspected).unwrap();
            let grant = self.memory.authority().approve_import(need).unwrap();
            unsafe { self.imports.map_approved(inspected, grant).unwrap() };
        }
        pub(crate) fn image_grant(&self, size: FrameSize) -> CaptureGrant {
            self.admission.bind_contract(None).unwrap();
            self.memory.format(Some((VideoFormat::Bgra, size))).unwrap();
            let output = HostOutputPlan::new(HostContract::Legacy, size, HostCodecPlan::Tiles).unwrap();
            let plan = self.memory.capture_plan(&output).unwrap().unwrap();
            let grant = self.memory.authority().approve(plan).unwrap();
            self.memory.activate(grant.clone()).unwrap();
            self.latest.admitted_locked();
            grant
        }
        pub(crate) fn native_live(&self) -> u64 {
            let state = self.imports.state.lock().unwrap();
            let mut total = 0;
            let mut node = state.head.as_ref();
            while let Some(owner) = node {
                total += owner.grant.control_grant().live_bytes().unwrap();
                node = owner.next.as_ref();
            }
            total
        }
        pub(crate) fn base(&self) -> usize {
            self.imports.state.lock().unwrap().head.as_ref().unwrap().base
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.imports.close();
            self.memory.stop();
            unsafe {
                self.imports.reap_release_handles();
                fixture::destroy();
                self.imports.finish_core();
            }
            let _ = &self.core;
        }
    }
}
