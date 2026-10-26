//! Fixed stream-view table. Mapping ownership lives in the Core import table,
//! not in the earlier guessed count/stride pool.
use super::*;
use crate::screen::core::capture_admission::CaptureGeneration;
#[derive(Clone, Copy)]
struct View {
    pointer: usize,
    generation: CaptureGeneration,
}
pub(super) struct BufferOwners {
    views: [Option<View>; 64],
}
impl Default for BufferOwners {
    fn default() -> Self {
        Self { views: [None; 64] }
    }
}
impl BufferOwners {
    pub(super) unsafe fn add(
        &mut self,
        buffer: *mut PwBuffer,
        generation: CaptureGeneration,
        imports: &imports::ImportController,
    ) -> io::Result<()> {
        if self
            .views
            .iter()
            .flatten()
            .any(|view| view.pointer == buffer as usize)
        {
            return Err(io::Error::other("pipewire duplicate buffer identity"));
        }
        imports.validate_buffer(buffer)?;
        let slot = self
            .views
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or_else(|| io::Error::other("pipewire exceeds stream API's 64 view slots"))?;
        *slot = Some(View {
            pointer: buffer as usize,
            generation,
        });
        Ok(())
    }
    pub(super) fn remove(&mut self, buffer: *mut PwBuffer) {
        if let Some(slot) = self
            .views
            .iter_mut()
            .find(|entry| entry.is_some_and(|view| view.pointer == buffer as usize))
        {
            *slot = None;
        }
        // remove_buffer precedes native unmap. No import ticket is released here.
    }
    pub(super) fn contains(&self, buffer: *mut PwBuffer, generation: CaptureGeneration) -> bool {
        self.views
            .iter()
            .flatten()
            .any(|view| view.pointer == buffer as usize && view.generation == generation)
    }
    pub(super) fn clear(&mut self) {
        self.views.fill(None);
    }
}
pub(super) unsafe extern "C" fn on_add_buffer(pointer: *mut c_void, buffer: *mut PwBuffer) {
    guarded(pointer, |data| {
        let result = unsafe {
            data.owners
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .add(buffer, data.memory.generation(), &data.imports)
        };
        if let Err(error) = result {
            data.fail(error.to_string());
        }
    });
}
pub(super) unsafe extern "C" fn on_remove_buffer(pointer: *mut c_void, buffer: *mut PwBuffer) {
    guarded(pointer, |data| {
        data.owners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(buffer)
    });
}
/// stream.c::pw_stream_connect uses PIPEWIRE_NODE to override target.object.
pub(super) fn check_target_override(value: Option<&std::ffi::OsStr>) -> io::Result<()> {
    if value.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "pipewire_target_override_not_allowed",
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod fixture {
    use super::*;
    use crate::screen::os::pipewire_abi::memory::{
        CoreEvents, MemBlock, MemMap, MemoryApi, MempoolEvents,
    };
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::sync::{Mutex, OnceLock};
    pub(crate) static SERIAL: Mutex<()> = Mutex::new(());
    struct Block {
        id: u32,
        block: usize,
        map: usize,
        base: usize,
        length: usize,
        live: bool,
        views: usize,
    }
    #[derive(Default)]
    struct State {
        blocks: Vec<Block>,
        data: usize,
        events: usize,
        pool_data: usize,
        pool_events: usize,
        pub paused: bool,
        pub maps: usize,
        pub fail_pause: bool,
        pub fail_map: bool,
        pub resume_add: Option<(u32, i32, u32)>,
    }
    fn state() -> &'static Mutex<State> {
        static STATE: OnceLock<Mutex<State>> = OnceLock::new();
        STATE.get_or_init(|| Mutex::new(State::default()))
    }
    pub(crate) fn reset() {
        *state().lock().unwrap() = State::default();
    }
    pub(crate) fn observed() -> (bool, usize) {
        let s = state().lock().unwrap();
        (s.paused, s.maps)
    }
    pub(crate) fn fail_pause() {
        state().lock().unwrap().fail_pause = true;
    }
    pub(crate) fn fail_map() {
        state().lock().unwrap().fail_map = true;
    }
    pub(crate) fn resume_add(id: u32, fd: i32, flags: u32) {
        state().lock().unwrap().resume_add = Some((id, fd, flags));
    }
    pub(crate) fn fd(length: i64) -> OwnedFd {
        let fd =
            unsafe { libc::memfd_create(c"capture-import-fixture".as_ptr(), libc::MFD_CLOEXEC) };
        assert!(fd >= 0);
        assert_eq!(unsafe { libc::ftruncate(fd, length as libc::off_t) }, 0);
        unsafe { OwnedFd::from_raw_fd(fd) }
    }
    pub(crate) unsafe fn add(id: u32, fd: &OwnedFd, flags: u32) {
        add_raw(id, fd.as_raw_fd(), flags);
    }
    unsafe fn add_raw(id: u32, fd: c_int, flags: u32) {
        let block = Box::into_raw(Box::new(MemBlock {
            pool: 1usize as *mut c_void,
            id,
            refs: 1,
            flags,
            type_: 2,
            fd,
            size: 0,
            map: std::ptr::null_mut(),
        }));
        let (data, events) = {
            let mut s = state().lock().unwrap();
            s.blocks.push(Block {
                id,
                block: block as usize,
                map: 0,
                base: 0,
                length: 0,
                live: true,
                views: 0,
            });
            (s.data, s.events)
        };
        let events = &*(events as *const CoreEvents);
        events.add_mem.unwrap()(data as *mut c_void, id, 2, fd, flags);
    }
    pub(crate) unsafe fn remove(id: u32) {
        let (data, events) = {
            let mut s = state().lock().unwrap();
            let b = s.blocks.iter_mut().find(|b| b.id == id && b.live).unwrap();
            b.live = false;
            (s.data, s.events)
        };
        (*(events as *const CoreEvents)).remove_mem.unwrap()(data as *mut c_void, id);
    }
    pub(crate) fn library_view(id: u32, offset: usize, bytes: usize) -> *mut c_void {
        let mut s = state().lock().unwrap();
        // Mock process_remote: even an already received port_use_buffers is
        // not dispatched while Core delivery is paused.
        if s.paused {
            return std::ptr::null_mut();
        }
        let b = s.blocks.iter_mut().find(|b| b.id == id && b.live).unwrap();
        if b.base == 0 || offset.checked_add(bytes).map_or(true, |end| end > b.length) {
            return std::ptr::null_mut();
        }
        b.views += 1;
        (b.base + offset) as *mut c_void
    }
    pub(crate) fn views(id: u32, count: usize) {
        state()
            .lock()
            .unwrap()
            .blocks
            .iter_mut()
            .find(|b| b.id == id)
            .unwrap()
            .views = count;
    }
    pub(crate) unsafe fn release_views(id: u32) {
        let block = {
            let mut s = state().lock().unwrap();
            let b = s.blocks.iter_mut().find(|b| b.id == id).unwrap();
            b.views = 0;
            b.block
        };
        finish(block, false);
    }
    unsafe fn finish(block: usize, forced: bool) {
        let ended = {
            let mut s = state().lock().unwrap();
            let Some(index) = s.blocks.iter().position(|b| b.block == block) else {
                return;
            };
            let b = &s.blocks[index];
            if !forced && (b.live || b.views != 0 || b.map != 0) {
                return;
            }
            let b = s.blocks.remove(index);
            (b, s.pool_data, s.pool_events)
        };
        let (b, data, events) = ended;
        // Deliberately notify before unmap to expose premature ticket release.
        if (*(b.block as *mut MemBlock)).flags & (1 << 5) == 0 {
            (*(events as *const MempoolEvents)).removed.unwrap()(
                data as *mut c_void,
                b.block as *mut MemBlock,
            );
        }
        if b.map != 0 {
            drop(Box::from_raw(b.map as *mut MemMap));
        }
        if b.base != 0 {
            libc::munmap(b.base as *mut c_void, b.length);
        }
        drop(Box::from_raw(b.block as *mut MemBlock));
    }
    pub(crate) unsafe fn destroy() {
        let (data, events) = {
            let s = state().lock().unwrap();
            (s.pool_data, s.pool_events)
        };
        (*(events as *const MempoolEvents)).destroy.unwrap()(data as *mut c_void);
        loop {
            let block = state().lock().unwrap().blocks.first().map(|b| b.block);
            match block {
                Some(b) => finish(b, true),
                None => break,
            }
        }
    }
    unsafe extern "C" fn paused(_core: *mut c_void, value: bool) -> c_int {
        let next = {
            let mut s = state().lock().unwrap();
            if s.fail_pause {
                return -22;
            }
            s.paused = value;
            if value {
                None
            } else {
                s.resume_add.take()
            }
        };
        if let Some((id, fd, flags)) = next {
            add_raw(id, fd, flags);
        }
        0
    }
    unsafe extern "C" fn pool(_core: *mut c_void) -> *mut c_void {
        1usize as *mut c_void
    }
    unsafe extern "C" fn find_id(_pool: *mut c_void, id: u32) -> *mut MemBlock {
        state()
            .lock()
            .unwrap()
            .blocks
            .iter()
            .find(|b| b.id == id && b.live)
            .map_or(std::ptr::null_mut(), |b| b.block as *mut MemBlock)
    }
    unsafe extern "C" fn find_fd(_pool: *mut c_void, fd: c_int) -> *mut MemBlock {
        state()
            .lock()
            .unwrap()
            .blocks
            .iter()
            .find(|b| (*(b.block as *mut MemBlock)).fd == fd)
            .map_or(std::ptr::null_mut(), |b| b.block as *mut MemBlock)
    }
    unsafe extern "C" fn find_ptr(_pool: *mut c_void, ptr: *const c_void) -> *mut MemBlock {
        state()
            .lock()
            .unwrap()
            .blocks
            .iter()
            .find(|b| b.base != 0 && (ptr as usize) >= b.base && (ptr as usize) < b.base + b.length)
            .map_or(std::ptr::null_mut(), |b| b.block as *mut MemBlock)
    }
    unsafe extern "C" fn map(
        block: *mut MemBlock,
        flags: u32,
        offset: u32,
        size: u32,
        _tag: *mut u32,
    ) -> *mut MemMap {
        if state().lock().unwrap().fail_map {
            return std::ptr::null_mut();
        }
        let prot = if flags & 2 != 0 {
            libc::PROT_READ | libc::PROT_WRITE
        } else {
            libc::PROT_READ
        };
        let ptr = libc::mmap(
            std::ptr::null_mut(),
            size as usize,
            prot,
            libc::MAP_SHARED,
            (*block).fd,
            offset as libc::off_t,
        );
        if ptr == libc::MAP_FAILED {
            return std::ptr::null_mut();
        }
        let map = Box::into_raw(Box::new(MemMap {
            block,
            ptr,
            flags,
            offset,
            size,
            tag: [0; 5],
        }));
        let mut s = state().lock().unwrap();
        s.maps += 1;
        let b = s
            .blocks
            .iter_mut()
            .find(|b| b.block == block as usize)
            .unwrap();
        b.map = map as usize;
        b.base = ptr as usize;
        b.length = size as usize;
        map
    }
    unsafe extern "C" fn free_map(map: *mut MemMap) -> c_int {
        let block = (*map).block as usize;
        {
            let mut s = state().lock().unwrap();
            s.blocks.iter_mut().find(|b| b.block == block).unwrap().map = 0;
        }
        drop(Box::from_raw(map));
        finish(block, false);
        0
    }
    unsafe extern "C" fn listen(
        _pool: *mut c_void,
        _hook: *mut SpaHook,
        events: *const MempoolEvents,
        data: *mut c_void,
    ) {
        let mut s = state().lock().unwrap();
        s.pool_data = data as usize;
        s.pool_events = events as usize;
    }
    pub(crate) unsafe extern "C" fn core_listen(
        _core: *mut c_void,
        _hook: *mut SpaHook,
        events: *const c_void,
        data: *mut c_void,
    ) -> c_int {
        let mut s = state().lock().unwrap();
        s.data = data as usize;
        s.events = events as usize;
        0
    }
    pub(crate) fn api() -> MemoryApi {
        MemoryApi {
            set_paused: paused,
            get_mempool: pool,
            find_id,
            find_fd,
            find_ptr,
            map,
            free_map,
            add_listener: listen,
        }
    }
}
