//! The part of PipeWire's C interface the screen stream uses (SCR-M5,
//! U19-5), loaded at run time from `libpipewire-0.3.so.0` with `libloading`.
//! The program links no PipeWire library: without it (or with a library that
//! lacks a symbol) loading fails with an error, the portal source reports
//! `pipewire_missing` and everything else keeps working.
//!
//! The structure layouts follow `pipewire/stream.h`, `spa/buffer/buffer.h`
//! and `spa/utils/hook.h` of the 0.3 series (assumption A11: stable over
//! 0.3.x to 1.6; the structures are only read, never allocated, except the
//! hook, which is reserved with room to spare). The host process isolates a
//! mismatch from the daemon.
use std::ffi::{c_char, c_int, c_void};
use std::io;
use std::sync::{Arc, OnceLock};

use libloading::Library;

/// The shared object of PipeWire's 0.3 API (`soversion` 0).
pub(crate) const LIBRARY_NAME: &str = "libpipewire-0.3.so.0";
pub(crate) const PW_DIRECTION_INPUT: c_int = 0;
pub(crate) const PW_STREAM_FLAG_AUTOCONNECT: u32 = 1 << 0;
pub(crate) const PW_STREAM_FLAG_INACTIVE: u32 = 1 << 1;
pub(crate) const PW_STREAM_FLAG_MAP_BUFFERS: u32 = 1 << 2;
pub(crate) const PW_STREAM_FLAG_DONT_RECONNECT: u32 = 1 << 7;
pub(crate) const PW_ID_ANY: u32 = u32::MAX;
pub(crate) const SPA_PARAM_FORMAT: u32 = 4;
pub(crate) const PW_STREAM_STATE_ERROR: c_int = -1;
/// `SPA_CHUNK_FLAG_CORRUPTED`: the chunk's data must not be looked at.
pub(crate) const SPA_CHUNK_FLAG_CORRUPTED: i32 = 1 << 0;
/// `PW_VERSION_STREAM_EVENTS`.
pub(crate) const PW_VERSION_STREAM_EVENTS: u32 = 2;

/// `struct spa_hook`: the list link (two pointers), the callback table
/// (`funcs`, `data`), the removal callback and a private pointer. Four
/// words of reserve guard against a longer hook in a newer release.
#[repr(C)]
pub(crate) struct SpaHook {
    pub link: [*mut c_void; 2],
    pub funcs: *const c_void,
    pub data: *mut c_void,
    pub removed: Option<unsafe extern "C" fn(*mut SpaHook)>,
    pub private: *mut c_void,
    reserve: [usize; 4],
}

impl SpaHook {
    pub(crate) fn zeroed() -> Self {
        Self {
            link: [std::ptr::null_mut(); 2],
            funcs: std::ptr::null(),
            data: std::ptr::null_mut(),
            removed: None,
            private: std::ptr::null_mut(),
            reserve: [0; 4],
        }
    }
}

/// `struct spa_pod` header.
#[repr(C)]
pub(crate) struct SpaPod {
    pub size: u32,
    pub type_: u32,
}

#[repr(C)]
pub(crate) struct SpaChunk {
    pub offset: u32,
    pub size: u32,
    pub stride: i32,
    pub flags: i32,
}

#[repr(C)]
pub(crate) struct SpaData {
    pub type_: u32,
    pub flags: u32,
    pub fd: i64,
    pub mapoffset: u32,
    pub maxsize: u32,
    pub data: *mut c_void,
    pub chunk: *mut SpaChunk,
}

#[repr(C)]
pub(crate) struct SpaMeta {pub type_:u32,pub size:u32,pub data:*mut c_void}
#[repr(C)]
pub(crate) struct SpaBuffer {
    pub n_metas: u32,
    pub n_datas: u32,
    pub metas: *mut SpaMeta,
    pub datas: *mut SpaData,
}

/// `struct pw_buffer`; only `buffer` is read.
#[repr(C)]
pub(crate) struct PwBuffer {
    pub buffer: *mut SpaBuffer,
    pub user_data: *mut c_void,
    pub size: u64,
    pub requested: u64,
}

type Callback = Option<unsafe extern "C" fn(*mut c_void)>;

/// `struct pw_stream_events` version 2, the fields in header order; only
/// `state_changed`, `param_changed` and `process` are set.
#[repr(C)]
pub(crate) struct PwStreamEvents {
    pub version: u32,
    pub destroy: Callback,
    pub state_changed: Option<unsafe extern "C" fn(*mut c_void, c_int, c_int, *const c_char)>,
    pub control_info: Option<unsafe extern "C" fn(*mut c_void, u32, *const c_void)>,
    pub io_changed: Option<unsafe extern "C" fn(*mut c_void, u32, *mut c_void, u32)>,
    pub param_changed: Option<unsafe extern "C" fn(*mut c_void, u32, *const SpaPod)>,
    pub add_buffer: Option<unsafe extern "C" fn(*mut c_void, *mut PwBuffer)>,
    pub remove_buffer: Option<unsafe extern "C" fn(*mut c_void, *mut PwBuffer)>,
    pub process: Callback,
    pub drained: Callback,
    pub command: Option<unsafe extern "C" fn(*mut c_void, *const c_void)>,
    pub trigger_done: Callback,
}

/// The loaded functions; the library stays loaded as long as this lives.
pub(crate) struct PipewireApi {
    pub memory: memory::MemoryApi,
    pub init: unsafe extern "C" fn(*mut c_int, *mut *mut *mut c_char),
    pub thread_loop_new: unsafe extern "C" fn(*const c_char, *const c_void) -> *mut c_void,
    pub thread_loop_get_loop: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    pub thread_loop_start: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub thread_loop_stop: unsafe extern "C" fn(*mut c_void),
    pub thread_loop_destroy: unsafe extern "C" fn(*mut c_void),
    pub thread_loop_lock: unsafe extern "C" fn(*mut c_void),
    pub thread_loop_unlock: unsafe extern "C" fn(*mut c_void),
    pub context_new: unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> *mut c_void,
    pub context_destroy: unsafe extern "C" fn(*mut c_void),
    pub context_connect_fd:
        unsafe extern "C" fn(*mut c_void, c_int, *mut c_void, usize) -> *mut c_void,
    pub core_disconnect: unsafe extern "C" fn(*mut c_void) -> c_int,
    /// `pw_properties_new(key, value, …, NULL)`.
    pub properties_new: unsafe extern "C" fn(*const c_char, ...) -> *mut c_void,
    pub stream_new: unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void) -> *mut c_void,
    pub stream_add_listener:
        unsafe extern "C" fn(*mut c_void, *mut SpaHook, *const PwStreamEvents, *mut c_void),
    pub stream_connect:
        unsafe extern "C" fn(*mut c_void, c_int, u32, u32, *mut *const SpaPod, u32) -> c_int,
    pub stream_update_params: unsafe extern "C" fn(*mut c_void,*mut *const SpaPod,u32)->c_int,
    pub stream_dequeue_buffer: unsafe extern "C" fn(*mut c_void) -> *mut PwBuffer,
    pub stream_queue_buffer: unsafe extern "C" fn(*mut c_void, *mut PwBuffer) -> c_int,
    pub stream_set_active: unsafe extern "C" fn(*mut c_void,bool) -> c_int,
    pub stream_disconnect: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub stream_destroy: unsafe extern "C" fn(*mut c_void),
    /// The version string, for the log only.
    pub get_library_version: unsafe extern "C" fn() -> *const c_char,
    _library: Library,
}

fn symbol<T: Copy>(library: &Library, name: &str) -> io::Result<T> {
    let mut bytes = name.as_bytes().to_vec();
    bytes.push(0);
    // SAFETY: the type `T` is the C signature of the named PipeWire function;
    // the caller (`load_from`) spells each one out below.
    let found = unsafe { library.get::<T>(&bytes) };
    found
        .map(|symbol| *symbol)
        .map_err(|error| io::Error::other(format!("{LIBRARY_NAME}: {name}: {error}")))
}

/// Loads PipeWire from the shared object `name`.
pub(crate) fn load_from(name: &str) -> io::Result<PipewireApi> {
    // SAFETY: loading PipeWire runs its initializers, which only register
    // the library's own state.
    let library = unsafe { Library::new(name) }
        .map_err(|error| io::Error::new(io::ErrorKind::NotFound, format!("{name}: {error}")))?;
    let api = PipewireApi {
        memory: memory::MemoryApi::load(&library)?,
        init: symbol(&library, "pw_init")?,
        thread_loop_new: symbol(&library, "pw_thread_loop_new")?,
        thread_loop_get_loop: symbol(&library, "pw_thread_loop_get_loop")?,
        thread_loop_start: symbol(&library, "pw_thread_loop_start")?,
        thread_loop_stop: symbol(&library, "pw_thread_loop_stop")?,
        thread_loop_destroy: symbol(&library, "pw_thread_loop_destroy")?,
        thread_loop_lock: symbol(&library, "pw_thread_loop_lock")?,
        thread_loop_unlock: symbol(&library, "pw_thread_loop_unlock")?,
        context_new: symbol(&library, "pw_context_new")?,
        context_destroy: symbol(&library, "pw_context_destroy")?,
        context_connect_fd: symbol(&library, "pw_context_connect_fd")?,
        core_disconnect: symbol(&library, "pw_core_disconnect")?,
        properties_new: symbol(&library, "pw_properties_new")?,
        stream_new: symbol(&library, "pw_stream_new")?,
        stream_add_listener: symbol(&library, "pw_stream_add_listener")?,
        stream_connect: symbol(&library, "pw_stream_connect")?,
        stream_update_params: symbol(&library,"pw_stream_update_params")?,
        stream_dequeue_buffer: symbol(&library, "pw_stream_dequeue_buffer")?,
        stream_queue_buffer: symbol(&library, "pw_stream_queue_buffer")?,
        stream_set_active: symbol(&library,"pw_stream_set_active")?,
        stream_disconnect: symbol(&library, "pw_stream_disconnect")?,
        stream_destroy: symbol(&library, "pw_stream_destroy")?,
        get_library_version: symbol(&library, "pw_get_library_version")?,
        _library: library,
    };
    Ok(api)
}

/// PipeWire of this system, loaded once per process; the failure is kept
/// too, so a missing library is looked for only once.
pub(crate) fn load() -> Result<Arc<PipewireApi>, String> {
    static API: OnceLock<Result<Arc<PipewireApi>, String>> = OnceLock::new();
    API.get_or_init(|| {
        let api = load_from(LIBRARY_NAME).map_err(|error| error.to_string())?;
        // SAFETY: `pw_init` accepts null arguments (documented).
        unsafe { (api.init)(std::ptr::null_mut(), std::ptr::null_mut()) };
        Ok(Arc::new(api))
    })
    .clone()
}

/// PipeWire from the shared object `name`: the cached one of this system
/// for the default name, else a fresh load (tests with a wrong name).
pub(crate) fn load_named(name: &str) -> Result<Arc<PipewireApi>, String> {
    if name == LIBRARY_NAME {
        return load();
    }
    let api = load_from(name).map_err(|error| error.to_string())?;
    // SAFETY: `pw_init` accepts null arguments (documented).
    unsafe { (api.init)(std::ptr::null_mut(), std::ptr::null_mut()) };
    Ok(Arc::new(api))
}

/// Whether PipeWire can be loaded from `name` (the `pipewire_missing`
/// limit).
pub(crate) fn available(name: &str) -> bool {
    if name == LIBRARY_NAME {
        return load().is_ok();
    }
    load_from(name).is_ok()
}


// Public inline SPA interfaces from PipeWire 0.3.65 core.h/port.h and hook.h.
// The callbacks' data pointer, not the interface address, is the first
// method argument. These layouts avoid private proxy or marshal internals.
#[repr(C)]
pub(super) struct SpaInterface {
    pub type_: *const c_char, pub version:u32,
    pub funcs:*const c_void, pub data:*mut c_void,
}
#[repr(C)]
pub(super) struct CoreMethods {
    pub version:u32,
    pub add_listener:Option<unsafe extern "C" fn(*mut c_void,*mut SpaHook,*const c_void,*mut c_void)->c_int>,
    pub hello:Option<unsafe extern "C" fn(*mut c_void,u32)->c_int>,
    pub sync:Option<unsafe extern "C" fn(*mut c_void,u32,c_int)->c_int>,
    pub pong:Option<unsafe extern "C" fn(*mut c_void,u32,c_int)->c_int>,
    pub error:Option<unsafe extern "C" fn(*mut c_void,u32,c_int,c_int,*const c_char)->c_int>,
    pub get_registry:Option<unsafe extern "C" fn(*mut c_void,u32,usize)->*mut c_void>,
}

#[path="pipewire_memory_abi.rs"]
pub(crate) mod memory;
