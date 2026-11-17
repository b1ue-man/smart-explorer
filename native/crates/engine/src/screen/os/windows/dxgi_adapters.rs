//! The hardware graphics adapters of this system and the driver libraries
//! of their vendors (plan 2026-11-14 „Bildkette“ 4.10a, merge W1 of the
//! Windows packages BK2a, BK6 and BK12).
//!
//! - `adapters`: every hardware adapter (`CreateDXGIFactory1`,
//!   `EnumAdapters1`, software adapters left out) with its PCI vendor id,
//!   its LUID packed as `HighPart << 32 | LowPart` (the packing of
//!   `DeviceSet::adapter_luid`) and whether a desktop output hangs on it.
//!   Desktop Duplication runs on the adapter of its output (the capture
//!   creates its device there, `capture_dxgi.rs`), so the adapters with an
//!   output come first: their encoders are the ones a capture can feed
//!   without crossing adapters (hybrid laptops, BK2a/BK12 E2).
//! - `driver_library`: a vendor library such as `nvEncodeAPI64.dll` or
//!   `amfrt64.dll` from System32 first, as both references load it (Sunshine
//!   `nvenc_dynamic_factory.cpp`: `LoadLibraryEx(…, LOAD_LIBRARY_SEARCH_SYSTEM32)`;
//!   FFmpeg n7.1 `compat/w32dlfcn.h`: application directory and System32),
//!   else from the driver store folder of the vendor's adapter, the full
//!   path the kernel reports for it (`D3DKMTQueryAdapterInfo` with
//!   `KMTQAITYPE_QUERYREGISTRY` and `D3DDDI_QUERYREGISTRY_DRIVERSTOREPATH`,
//!   „<SystemDrive>:\system32\DriverStore\FileRepository\<DriverString>“,
//!   Microsoft Learn `d3dukmdt` `_D3DDDI_QUERYREGISTRY_TYPE`, read on
//!   2026-11-14; the two-call protocol with `BUFFER_OVERFLOW` as the Khronos
//!   OpenCL ICD loader `icd_windows_dxgk.c` does it). Never the search path:
//!   the full path is loaded with `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR |
//!   LOAD_LIBRARY_SEARCH_SYSTEM32`, so its own dependencies come from the
//!   same driver folder or System32 only. The kernel's answer is not loaded
//!   as it stands: only its folder name below the store is taken and put
//!   below this system's own `GetSystemDirectoryW` (`store_folder`), so a
//!   path without a drive (`\SystemRoot\System32\…`) or outside the store
//!   never reaches `LoadLibraryExW` (a rooted path without a drive would
//!   resolve on the current drive, where an unprivileged user may create
//!   the folder `\SystemRoot`).
//!
//! - `identity` (plan 2026-11-17 „Windows-Bild“ P3): PCI vendor, device
//!   and revision of an adapter and the version of its user-mode driver
//!   (`IDXGIAdapter::CheckInterfaceSupport` with `IDXGIDevice`), the key of
//!   the codec verdicts, read once per process.
//!
//! The D3DKMT entries are loaded from `gdi32.dll` in System32 at run time;
//! their structures are mirrored from the crate `windows` 0.58.0
//! (`Wdk::Graphics::Direct3D`: `D3DKMT_OPENADAPTERFROMLUID`,
//! `D3DKMT_QUERYADAPTERINFO`, `D3DKMT_CLOSEADAPTER`,
//! `D3DDDI_QUERYREGISTRY_INFO`, `KMTQAITYPE_QUERYREGISTRY` = 48,
//! `D3DDDI_QUERYREGISTRY_DRIVERSTOREPATH` = 2, status `SUCCESS` = 0,
//! `BUFFER_OVERFLOW` = 1), so no further crate feature is needed.
use std::ffi::{c_void, OsString};
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use libloading::os::windows::{
    Library, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows::core::Interface;
use windows::Win32::Foundation::LUID;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIDevice, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

/// One hardware adapter.
#[derive(Clone)]
pub(in crate::screen::os) struct Adapter {
    pub adapter: IDXGIAdapter1,
    pub vendor: u32,
    pub luid: LUID,
    /// A desktop output hangs on it (Desktop Duplication captures there).
    pub output: bool,
}

/// `LUID` packed as `DeviceSet::adapter_luid` packs it.
pub(in crate::screen::os) fn packed(luid: LUID) -> u64 {
    (u64::from(luid.HighPart as u32) << 32) | u64::from(luid.LowPart)
}

/// The hardware adapters, those with an output first, each group in the
/// order of `EnumAdapters1`.
pub(in crate::screen::os) fn adapters() -> Vec<Adapter> {
    // SAFETY: a plain DXGI factory; adapters are only enumerated and read.
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else {
        return Vec::new();
    };
    let software = DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32;
    let mut found = Vec::new();
    let mut index = 0;
    // SAFETY: enumeration of the live factory until it reports no adapter.
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(index) } {
        index += 1;
        // SAFETY: plain queries of the live adapter.
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        if desc.Flags & software != 0 {
            continue;
        }
        // SAFETY: as above; output 0 exists when any output does.
        let output = unsafe { adapter.EnumOutputs(0) }.is_ok();
        found.push(Adapter {
            adapter,
            vendor: desc.VendorId,
            luid: desc.AdapterLuid,
            output,
        });
    }
    order_by_output(&mut found, |adapter| adapter.output);
    found
}

/// `vendor:device:revision@a.b.c.d` of `adapter` (hexadecimal PCI ids; the
/// four 16-bit parts of the user-mode driver version where the driver
/// reports it).
fn identity(adapter: &IDXGIAdapter1) -> String {
    // SAFETY: plain queries of the live adapter.
    let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
        return String::new();
    };
    let base = format!(
        "{:04x}:{:04x}:{:02x}",
        desc.VendorId, desc.DeviceId, desc.Revision
    );
    // SAFETY: as above; the call only writes the version.
    match unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID) } {
        Ok(version) => {
            let part = |shift: u32| ((version as u64) >> shift) & 0xffff;
            format!("{base}@{}.{}.{}.{}", part(48), part(32), part(16), part(0))
        }
        Err(_) => base,
    }
}

/// Packed LUID, PCI vendor and identity of every hardware adapter, read
/// once per process (`adapters` order).
fn identities() -> &'static [(u64, u32, String)] {
    static FOUND: OnceLock<Vec<(u64, u32, String)>> = OnceLock::new();
    FOUND.get_or_init(|| {
        adapters()
            .iter()
            .map(|adapter| {
                let identity = identity(&adapter.adapter);
                (packed(adapter.luid), adapter.vendor, identity)
            })
            .collect()
    })
}

/// The identity of the adapter of the packed LUID `luid`; empty when no
/// hardware adapter has it.
pub(in crate::screen::os) fn identity_of(luid: u64) -> String {
    identities()
        .iter()
        .find(|(own, _, _)| *own == luid)
        .map(|(_, _, identity)| identity.clone())
        .unwrap_or_default()
}

/// The PCI vendor of the adapter of the packed LUID `luid`.
pub(in crate::screen::os) fn vendor_of_luid(luid: u64) -> Option<u32> {
    identities()
        .iter()
        .find(|(own, _, _)| *own == luid)
        .map(|(_, vendor, _)| *vendor)
}

/// The identity of the first hardware adapter of the PCI `vendor`.
pub(in crate::screen::os) fn identity_of_vendor(vendor: u32) -> String {
    identities()
        .iter()
        .find(|(_, own, _)| *own == vendor)
        .map(|(_, _, identity)| identity.clone())
        .unwrap_or_default()
}

/// The identity of the first hardware adapter (where a decoder's hardware
/// stage runs).
pub(in crate::screen::os) fn identity_of_first() -> String {
    identities()
        .first()
        .map(|(_, _, identity)| identity.clone())
        .unwrap_or_default()
}

/// Stable: the entries `output` names first (pure, for the task tests).
pub(in crate::screen::os) fn order_by_output<T>(list: &mut [T], output: impl Fn(&T) -> bool) {
    list.sort_by_key(|entry| !output(entry));
}

/// The first hardware adapter of the PCI `vendor` (an adapter with an
/// output first).
pub(in crate::screen::os) fn of_vendor(vendor: u32) -> Option<Adapter> {
    adapters()
        .into_iter()
        .find(|adapter| adapter.vendor == vendor)
}

// The kernel reads and writes these C layouts; every field exists for its
// offset.
#[allow(dead_code)]
#[repr(C)]
struct OpenFromLuid {
    luid: LUID,
    adapter: u32,
}

#[allow(dead_code)]
#[repr(C)]
struct QueryAdapterInfo {
    adapter: u32,
    kind: i32,
    data: *mut c_void,
    size: u32,
}

#[allow(dead_code)]
#[repr(C)]
struct CloseAdapter {
    adapter: u32,
}

/// `D3DDDI_QUERYREGISTRY_INFO` up to its output union (8-aligned, 552
/// bytes; the output string starts at offset 544).
#[allow(dead_code)]
#[repr(C)]
#[derive(Clone, Copy)]
pub(in crate::screen::os) struct QueryRegistryInfo {
    pub query_type: i32,
    pub query_flags: u32,
    pub value_name: [u16; 260],
    pub value_type: u32,
    pub physical_adapter_index: u32,
    pub output_value_size: u32,
    pub status: i32,
    pub output: u64,
}

pub(in crate::screen::os) const KMTQAITYPE_QUERYREGISTRY: i32 = 48;
pub(in crate::screen::os) const QUERY_DRIVER_STORE_PATH: i32 = 2;
pub(in crate::screen::os) const STATUS_SUCCESS: i32 = 0;
pub(in crate::screen::os) const STATUS_BUFFER_OVERFLOW: i32 = 1;
/// Offset of `OutputString` in `D3DDDI_QUERYREGISTRY_INFO`.
pub(in crate::screen::os) const OUTPUT_OFFSET: usize = 544;

type OpenFn = unsafe extern "system" fn(*mut OpenFromLuid) -> i32;
type QueryFn = unsafe extern "system" fn(*mut QueryAdapterInfo) -> i32;
type CloseFn = unsafe extern "system" fn(*const CloseAdapter) -> i32;

/// The UTF-16 path a successful query wrote behind its header (`words`
/// holds the header and `size` bytes of output; pure, for the task tests).
pub(in crate::screen::os) fn output_path(words: &[u64], size: u32) -> Option<PathBuf> {
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_ne_bytes()).collect();
    let end = OUTPUT_OFFSET.checked_add(size as usize)?.min(bytes.len());
    let text: Vec<u16> = bytes
        .get(OUTPUT_OFFSET..end)?
        .chunks_exact(2)
        .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    (!text.is_empty()).then(|| PathBuf::from(OsString::from_wide(&text)))
}

/// Whether `path` is a folder of the driver store (the only place a driver
/// library is taken from besides System32).
pub(in crate::screen::os) fn in_driver_store(path: &Path) -> bool {
    let text = path
        .to_string_lossy()
        .to_ascii_lowercase()
        .replace('/', "\\");
    text.contains("\\system32\\driverstore\\filerepository\\") && !text.contains("\\..")
}

/// The system directory of this Windows (`GetSystemDirectoryW`), asked of
/// the system and never read from the environment.
fn system_directory() -> Option<PathBuf> {
    // SAFETY: a length query without a buffer.
    let needed = unsafe { GetSystemDirectoryW(std::ptr::null_mut(), 0) };
    if needed == 0 {
        return None;
    }
    let mut buffer = vec![0u16; needed as usize];
    // SAFETY: `buffer` holds the `needed` units the call may write.
    let written = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), needed) } as usize;
    (written != 0 && written < buffer.len())
        .then(|| PathBuf::from(OsString::from_wide(&buffer[..written])))
}

/// The folder the kernel named, as a folder below the driver store of
/// `system32` (the system directory): `<system32>\DriverStore\FileRepository\…`.
/// The kernel may name it with the NT prefix `\SystemRoot\System32\…`; that
/// form is rooted without a drive, so a load would resolve it on the current
/// drive, where an unprivileged user may create a folder `\SystemRoot`. Only
/// the remainder behind the store is taken and put below `system32`; a path
/// outside the store, with `.` or `..` parts or a drive inside is refused
/// (pure, for the task tests).
pub(in crate::screen::os) fn store_folder(path: &Path, system32: &Path) -> Option<PathBuf> {
    let store = system32.join("DriverStore").join("FileRepository");
    let text = path.to_string_lossy().replace('/', "\\");
    // ASCII lower case keeps every byte offset of `text`.
    let lower = text.to_ascii_lowercase();
    let own = format!(
        "{}\\",
        store
            .to_string_lossy()
            .replace('/', "\\")
            .to_ascii_lowercase()
    );
    let nt = "\\systemroot\\system32\\driverstore\\filerepository\\";
    let skip = if lower.starts_with(own.as_str()) {
        own.len()
    } else if lower.starts_with(nt) {
        nt.len()
    } else {
        return None;
    };
    let rest = text.get(skip..)?.trim_end_matches('\\');
    let plain = !rest.is_empty()
        && rest
            .split('\\')
            .all(|part| !part.is_empty() && part != "." && part != ".." && !part.contains(':'));
    plain.then(|| store.join(rest))
}

/// The driver store folder of the adapter `luid`.
fn driver_store(luid: LUID) -> io::Result<PathBuf> {
    // SAFETY: gdi32.dll from System32 only; its initialisers are the system's.
    let gdi = unsafe { Library::load_with_flags("gdi32.dll", LOAD_LIBRARY_SEARCH_SYSTEM32) }
        .map_err(io::Error::other)?;
    // SAFETY: the signatures of d3dkmthk.h as the crate `windows` binds them.
    let (open, query, close) = unsafe {
        (
            *gdi.get::<OpenFn>(b"D3DKMTOpenAdapterFromLuid\0")
                .map_err(io::Error::other)?,
            *gdi.get::<QueryFn>(b"D3DKMTQueryAdapterInfo\0")
                .map_err(io::Error::other)?,
            *gdi.get::<CloseFn>(b"D3DKMTCloseAdapter\0")
                .map_err(io::Error::other)?,
        )
    };
    let mut opened = OpenFromLuid { luid, adapter: 0 };
    // SAFETY: a writable structure naming a LUID of `EnumAdapters1`.
    if unsafe { open(&mut opened) } != 0 {
        return Err(io::Error::other("D3DKMTOpenAdapterFromLuid failed"));
    }
    let header = std::mem::size_of::<QueryRegistryInfo>();
    let ask = |extra: u32| -> io::Result<(Vec<u64>, QueryRegistryInfo)> {
        let bytes = header + extra as usize;
        let mut words = vec![0u64; bytes.div_ceil(8)];
        // SAFETY: `words` is 8-aligned and at least one header long.
        let info = unsafe { &mut *words.as_mut_ptr().cast::<QueryRegistryInfo>() };
        info.query_type = QUERY_DRIVER_STORE_PATH;
        let mut request = QueryAdapterInfo {
            adapter: opened.adapter,
            kind: KMTQAITYPE_QUERYREGISTRY,
            data: words.as_mut_ptr().cast(),
            size: u32::try_from(words.len() * 8).map_err(io::Error::other)?,
        };
        // SAFETY: an open adapter handle; the buffer outlives the call.
        let status = unsafe { query(&mut request) };
        // SAFETY: the header the call filled.
        let info = unsafe { *words.as_ptr().cast::<QueryRegistryInfo>() };
        if status != 0 {
            return Err(io::Error::other("D3DKMTQueryAdapterInfo failed"));
        }
        Ok((words, info))
    };
    let result = ask(0).and_then(|(words, info)| match info.status {
        STATUS_SUCCESS => Ok((words, info)),
        STATUS_BUFFER_OVERFLOW => ask(info.output_value_size),
        _ => Err(io::Error::other("driver store path refused")),
    });
    // SAFETY: the handle opened above, closed once.
    unsafe {
        close(&CloseAdapter {
            adapter: opened.adapter,
        })
    };
    let (words, info) = result?;
    if info.status != STATUS_SUCCESS {
        return Err(io::Error::other("driver store path refused"));
    }
    let system32 = system_directory()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no system directory"))?;
    output_path(&words, info.output_value_size)
        .filter(|path| in_driver_store(path))
        .and_then(|path| store_folder(&path, &system32))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no driver store path"))
}

/// `name` in the driver store folder `dir` or one folder below it (AMD
/// keeps its user-mode files in a build folder there [assumption
/// A-W1-STORE-DEPTH]).
pub(in crate::screen::os) fn find_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.is_file() {
        return Some(direct);
    }
    let mut below: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path().join(name))
        .filter(|path| path.is_file())
        .collect();
    below.sort();
    below.into_iter().next()
}

/// Loads the driver library `name` of the PCI `vendor`: System32 first,
/// else the driver store folder of the vendor's adapter (module header).
pub(in crate::screen::os) fn driver_library(vendor: u32, name: &str) -> io::Result<Library> {
    // SAFETY: only System32 is searched, so no planted DLL loads; the
    // initialisers of the driver library are the driver's own.
    let system = unsafe { Library::load_with_flags(name, LOAD_LIBRARY_SEARCH_SYSTEM32) };
    let first = match system {
        Ok(library) => return Ok(library),
        Err(error) => error,
    };
    let path = of_vendor(vendor)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no adapter of the vendor"))
        .and_then(|adapter| driver_store(adapter.luid))
        .and_then(|dir| {
            find_in(&dir, name).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, format!("{name} not in {dir:?}"))
            })
        })
        .map_err(|error| {
            io::Error::new(io::ErrorKind::NotFound, format!("{name}: {first}; {error}"))
        })?;
    let flags = LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32;
    // SAFETY: a full path in the driver store (written by the driver
    // installation, protected like System32); dependencies come from that
    // folder or System32 only.
    unsafe { Library::load_with_flags(&path, flags) }
        .map_err(|error| io::Error::new(io::ErrorKind::NotFound, format!("{path:?}: {error}")))
}
