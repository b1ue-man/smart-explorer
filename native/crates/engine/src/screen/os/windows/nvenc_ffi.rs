//! NVIDIA Video Codec SDK library of the Windows NVENC encoder (BK12,
//! B-22), loaded at run time from `nvEncodeAPI64.dll` in System32 (as
//! Sunshine `nvenc_dynamic_factory.cpp` loads it), else from the driver
//! store folder of the NVIDIA adapter by its full path
//! (`dxgi_adapters::driver_library`, merge W1); never through the search
//! path. Without the NVIDIA driver nothing is found and nothing changes.
//!
//! Constants, versions, structures and the function list are shared with
//! Linux (merge W2: `native_ffi/nvenc_shared.rs`, re-exported here, so every
//! `nvenc_ffi::` path of the Windows files stays); this file keeps what is
//! Windows' own: the library and its loading. `NVENCAPI` is `__stdcall` on
//! Windows (header lines 70-71); the shared entries are `extern "system"`.
use std::io;
use std::sync::{Arc, OnceLock};

use libloading::os::windows::Library;

pub(super) use crate::screen::os::nvenc_shared::abi::*;
pub(super) use crate::screen::os::nvenc_shared::api::*;

/// `nvEncodeAPI64.dll`: the 64-bit NVENC driver library (System32, else the
/// driver store folder of the NVIDIA adapter).
pub(super) const NVENC_LIBRARY: &str = "nvEncodeAPI64.dll";

/// The loaded NVENC entry points; the library stays loaded.
pub(super) struct NvApi {
    pub functions: NvEncFunctions,
    _nvenc: Library,
}

// SAFETY: the table holds plain function pointers of the thread-safe NVENC
// driver interface; it is never mutated after loading.
unsafe impl Send for NvApi {}
// SAFETY: as above; shared references only read the table.
unsafe impl Sync for NvApi {}

/// Loads `name` from System32 only (task tests).
#[cfg(test)]
pub(super) fn load_from(name: &str) -> io::Result<NvApi> {
    use libloading::os::windows::LOAD_LIBRARY_SEARCH_SYSTEM32;
    // SAFETY: only System32 is searched, so no planted DLL loads.
    let library = unsafe { Library::load_with_flags(name, LOAD_LIBRARY_SEARCH_SYSTEM32) }
        .map_err(|error| io::Error::new(io::ErrorKind::NotFound, format!("{name}: {error}")))?;
    instance(library)
}

/// The NVENC entry points of the loaded driver `library`; the initialisers
/// of the NVIDIA driver library are the driver's own.
fn instance(library: Library) -> io::Result<NvApi> {
    // SAFETY: the signatures of nvEncodeAPI.h lines 4272 and 4443.
    let (max_version, create) = unsafe {
        let max = *library
            .get::<MaxVersionFn>(b"NvEncodeAPIGetMaxSupportedVersion\0")
            .map_err(io::Error::other)?;
        let create = *library
            .get::<CreateInstanceFn>(b"NvEncodeAPICreateInstance\0")
            .map_err(io::Error::other)?;
        (max, create)
    };
    Ok(NvApi {
        functions: create_functions(max_version, create)?,
        _nvenc: library,
    })
}

impl NvTable for NvApi {
    fn table(&self) -> &NvEncFunctions {
        &self.functions
    }
}

/// NVENC of this system, loaded once per process (the failure is kept).
pub(super) fn load() -> Result<Arc<NvApi>, String> {
    static API: OnceLock<Result<Arc<NvApi>, String>> = OnceLock::new();
    API.get_or_init(|| {
        crate::screen::os::dxgi_adapters::driver_library(super::VENDOR_NVIDIA, NVENC_LIBRARY)
            .and_then(instance)
            .map(Arc::new)
            .map_err(|error| error.to_string())
    })
    .clone()
}
