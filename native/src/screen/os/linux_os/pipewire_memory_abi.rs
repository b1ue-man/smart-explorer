//! Public PipeWire 0.3.65 core.h/mem.h layouts, also checked at 1.6.9.
//! No private proxy, mapping, memblock implementation or activation layout.
use super::{c_char, c_int, c_void, symbol, Library, SpaHook};
use std::io;
#[repr(C)]
pub(crate) struct CoreEvents {
    pub version: u32,
    pub info: Option<unsafe extern "C" fn(*mut c_void, *const c_void)>,
    pub done: Option<unsafe extern "C" fn(*mut c_void, u32, c_int)>,
    pub ping: Option<unsafe extern "C" fn(*mut c_void, u32, c_int)>,
    pub error: Option<unsafe extern "C" fn(*mut c_void, u32, c_int, c_int, *const c_char)>,
    pub remove_id: Option<unsafe extern "C" fn(*mut c_void, u32)>,
    pub bound_id: Option<unsafe extern "C" fn(*mut c_void, u32, u32)>,
    pub add_mem: Option<unsafe extern "C" fn(*mut c_void, u32, u32, c_int, u32)>,
    pub remove_mem: Option<unsafe extern "C" fn(*mut c_void, u32)>,
}
#[repr(C)]
pub(crate) struct MemBlock {
    pub pool: *mut c_void,
    pub id: u32,
    pub refs: c_int,
    pub flags: u32,
    pub type_: u32,
    pub fd: c_int,
    pub size: u32,
    pub map: *mut MemMap,
}
#[repr(C)]
pub(crate) struct MemMap {
    pub block: *mut MemBlock,
    pub ptr: *mut c_void,
    pub flags: u32,
    pub offset: u32,
    pub size: u32,
    pub tag: [u32; 5],
}
#[repr(C)]
pub(crate) struct MempoolEvents {
    pub version: u32,
    pub destroy: Option<unsafe extern "C" fn(*mut c_void)>,
    pub added: Option<unsafe extern "C" fn(*mut c_void, *mut MemBlock)>,
    pub removed: Option<unsafe extern "C" fn(*mut c_void, *mut MemBlock)>,
}
#[derive(Clone, Copy)]
pub(crate) struct MemoryApi {
    pub set_paused: unsafe extern "C" fn(*mut c_void, bool) -> c_int,
    pub get_mempool: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    pub find_id: unsafe extern "C" fn(*mut c_void, u32) -> *mut MemBlock,
    pub find_fd: unsafe extern "C" fn(*mut c_void, c_int) -> *mut MemBlock,
    pub find_ptr: unsafe extern "C" fn(*mut c_void, *const c_void) -> *mut MemBlock,
    pub map: unsafe extern "C" fn(*mut MemBlock, u32, u32, u32, *mut u32) -> *mut MemMap,
    pub free_map: unsafe extern "C" fn(*mut MemMap) -> c_int,
    pub add_listener:
        unsafe extern "C" fn(*mut c_void, *mut SpaHook, *const MempoolEvents, *mut c_void),
}
impl MemoryApi {
    pub(super) fn load(library: &Library) -> io::Result<Self> {
        Ok(Self {
            set_paused: symbol(library, "pw_core_set_paused")?,
            get_mempool: symbol(library, "pw_core_get_mempool")?,
            find_id: symbol(library, "pw_mempool_find_id")?,
            find_fd: symbol(library, "pw_mempool_find_fd")?,
            find_ptr: symbol(library, "pw_mempool_find_ptr")?,
            map: symbol(library, "pw_memblock_map")?,
            free_map: symbol(library, "pw_memmap_free")?,
            add_listener: symbol(library, "pw_mempool_add_listener")?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FdFacts {
    pub dev: u64,
    pub inode: u64,
    pub length: u32,
    pub access: c_int,
    pub mapped: u64,
}
pub(crate) fn rounded_mapping(length: u64, page: u64) -> io::Result<u64> {
    if length == 0 || page == 0 {
        return Err(io::Error::other(
            "pipewire empty memory object or invalid page size",
        ));
    }
    let mapped = length
        .checked_add(page - 1)
        .and_then(|n| n.checked_div(page))
        .and_then(|n| n.checked_mul(page))
        .ok_or_else(|| io::Error::other("pipewire mapping size overflow"))?;
    if mapped > u32::MAX as u64 {
        return Err(io::Error::other("pipewire mapping exceeds public u32 API"));
    }
    Ok(mapped)
}
pub(crate) fn fd_facts(fd: c_int) -> io::Result<FdFacts> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: fstat initializes stat on success; the borrowed descriptor is never closed.
    if unsafe { libc::fstat(fd, stat.as_mut_ptr()) } < 0 {
        return Err(io::Error::last_os_error());
    }
    let stat = unsafe { stat.assume_init() };
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if stat.st_size <= 0 || page <= 0 {
        return Err(io::Error::other("pipewire invalid SHM length/page size"));
    }
    let length = u32::try_from(stat.st_size)
        .map_err(|_| io::Error::other("pipewire SHM exceeds mapping API"))?;
    let access = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if access < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(FdFacts {
        dev: stat.st_dev as u64,
        inode: stat.st_ino as u64,
        length,
        access: access & libc::O_ACCMODE,
        mapped: rounded_mapping(length as u64, page as u64)?,
    })
}
