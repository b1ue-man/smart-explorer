//! NVIDIA Video Codec SDK libraries of the Linux NVENC encoder (BK4,
//! B-24), loaded at run time from `libnvidia-encode.so.1` and the CUDA
//! driver `libcuda.so.1` (no link dependency; without the NVIDIA driver
//! nothing is found and nothing changes).
//!
//! Constants, versions, structures and the NVENC function list are shared
//! with Windows (merge W2: `native_ffi/nvenc_shared.rs`, re-exported here,
//! so every `nvenc_ffi::` path of the Linux files stays); this file keeps
//! what is Linux' own: both libraries, their loading and the CUDA functions
//! (`dynlink_cuda.h`/`dynlink_loader.h` of nv-codec-headers tag `n12.1.14.0`,
//! read on 2026-11-14). Drivers accept API 12.1 from branch 530 on
//! (assumption A-NV-DRIVER); an older driver reports a lower version and
//! NVENC stays unused.
use std::ffi::{c_int, c_uint};
use std::io;
use std::sync::{Arc, OnceLock};

use libloading::Library;

pub(super) use crate::screen::os::nvenc_shared::abi::*;
pub(super) use crate::screen::os::nvenc_shared::api::*;

pub(super) const NVENC_LIBRARY: &str = "libnvidia-encode.so.1";
pub(super) const CUDA_LIBRARY: &str = "libcuda.so.1";

/// The loaded NVENC and CUDA entry points; both libraries stay loaded.
pub(super) struct NvApi {
    pub functions: NvEncFunctions,
    pub cu_init: unsafe extern "C" fn(c_uint) -> c_int,
    pub cu_device_get_count: unsafe extern "C" fn(*mut c_int) -> c_int,
    pub cu_device_get: unsafe extern "C" fn(*mut c_int, c_int) -> c_int,
    pub cu_ctx_create: unsafe extern "C" fn(*mut Ptr, c_uint, c_int) -> c_int,
    pub cu_ctx_destroy: unsafe extern "C" fn(Ptr) -> c_int,
    pub cu_ctx_push: unsafe extern "C" fn(Ptr) -> c_int,
    pub cu_ctx_pop: unsafe extern "C" fn(*mut Ptr) -> c_int,
    _cuda: Library,
    _nvenc: Library,
}

// SAFETY: the function table holds plain function pointers of the
// thread-safe NVENC and CUDA driver interfaces; it is never mutated.
unsafe impl Send for NvApi {}
// SAFETY: as above; shared references only read the table.
unsafe impl Sync for NvApi {}

impl NvTable for NvApi {
    fn table(&self) -> &NvEncFunctions {
        &self.functions
    }
}

fn symbol<T: Copy>(library: &Library, file: &str, name: &str) -> io::Result<T> {
    let mut bytes = name.as_bytes().to_vec();
    bytes.push(0);
    // SAFETY: `T` is the C signature of the named function as declared in
    // nvEncodeAPI.h 12.1 or dynlink_cuda.h (spelled out in `load_from`).
    let found = unsafe { library.get::<T>(&bytes) };
    found
        .map(|symbol| *symbol)
        .map_err(|error| io::Error::other(format!("{file}: {name}: {error}")))
}

fn open(name: &str) -> io::Result<Library> {
    // SAFETY: loading the NVIDIA driver libraries runs only their own
    // initializers.
    unsafe { Library::new(name) }
        .map_err(|error| io::Error::new(io::ErrorKind::NotFound, format!("{name}: {error}")))
}

/// Loads NVENC from `nvenc` and CUDA from `cuda`.
pub(super) fn load_from(nvenc: &str, cuda: &str) -> io::Result<NvApi> {
    let nvenc_lib = open(nvenc)?;
    let cuda_lib = open(cuda)?;
    let max_version: MaxVersionFn = symbol(&nvenc_lib, nvenc, "NvEncodeAPIGetMaxSupportedVersion")?;
    let create: CreateInstanceFn = symbol(&nvenc_lib, nvenc, "NvEncodeAPICreateInstance")?;
    let functions = create_functions(max_version, create)?;
    Ok(NvApi {
        functions,
        cu_init: symbol(&cuda_lib, cuda, "cuInit")?,
        cu_device_get_count: symbol(&cuda_lib, cuda, "cuDeviceGetCount")?,
        cu_device_get: symbol(&cuda_lib, cuda, "cuDeviceGet")?,
        cu_ctx_create: symbol(&cuda_lib, cuda, "cuCtxCreate_v2")?,
        cu_ctx_destroy: symbol(&cuda_lib, cuda, "cuCtxDestroy_v2")?,
        cu_ctx_push: symbol(&cuda_lib, cuda, "cuCtxPushCurrent_v2")?,
        cu_ctx_pop: symbol(&cuda_lib, cuda, "cuCtxPopCurrent_v2")?,
        _cuda: cuda_lib,
        _nvenc: nvenc_lib,
    })
}

/// NVENC of this system, loaded once per process (the failure is kept).
pub(super) fn load() -> Result<Arc<NvApi>, String> {
    static API: OnceLock<Result<Arc<NvApi>, String>> = OnceLock::new();
    API.get_or_init(|| {
        let api = load_from(NVENC_LIBRARY, CUDA_LIBRARY).map_err(|error| error.to_string())?;
        // SAFETY: `cuInit(0)` is the documented first CUDA driver call.
        if unsafe { (api.cu_init)(0) } != 0 {
            return Err("cuInit failed".into());
        }
        Ok(Arc::new(api))
    })
    .clone()
}
