//! The FFmpeg DLLs at run time (plan 2026-11-17 P8): a minimal FFI of the
//! functions in `ffmpeg_plan::Symbol`, loaded with `LoadLibraryExW` from a
//! directory whose DLLs match their pins (`ffmpeg_runtime::verified_dir`),
//! with `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32`, so
//! the imports of each DLL come from beside it or from System32 and nowhere
//! else. Nothing is linked at build time: a program without the DLLs starts
//! and decodes with Media Foundation. Before the first decoder opens, the
//! loader checks the major versions (`avutil` 60, `avcodec` 62), the pixel
//! format and device type codes by name, and the layout fingerprints of a
//! fresh frame and packet (`ffmpeg_plan::fresh_frame_ok`, `fresh_packet_ok`);
//! any mismatch keeps FFmpeg off for the process. The libraries are never
//! unloaded.
use std::ffi::{c_char, c_int, c_void, CString};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use libloading::os::windows::{
    Library, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
};

use crate::screen::core::ffmpeg_plan::{
    self as plan, layout, OptionValue, PixelFormats, Symbol, AVCODEC_MAJOR, AVUTIL_MAJOR, DLLS,
};

pub(super) type Raw = *mut c_void;

type AnyFn = unsafe extern "C" fn();
type VersionFn = unsafe extern "C" fn() -> u32;
type AllocFn = unsafe extern "C" fn() -> Raw;
type FreeFn = unsafe extern "C" fn(*mut Raw);
type RefFn = unsafe extern "C" fn(Raw, *const c_void) -> c_int;
type UnrefFn = unsafe extern "C" fn(Raw);
type OptSetFn = unsafe extern "C" fn(Raw, *const c_char, *const c_char, c_int) -> c_int;
type OptSetIntFn = unsafe extern "C" fn(Raw, *const c_char, i64, c_int) -> c_int;
type OptGetIntFn = unsafe extern "C" fn(Raw, *const c_char, c_int, *mut i64) -> c_int;
type OptFindFn =
    unsafe extern "C" fn(Raw, *const c_char, *const c_char, c_int, c_int) -> *const c_void;
type HwAllocFn = unsafe extern "C" fn(c_int) -> Raw;
type HwInitFn = unsafe extern "C" fn(Raw) -> c_int;
type HwCreateFn = unsafe extern "C" fn(*mut Raw, c_int, *const c_char, Raw, c_int) -> c_int;
type ByNameFn = unsafe extern "C" fn(*const c_char) -> c_int;
type TransferFn = unsafe extern "C" fn(Raw, *const c_void, c_int) -> c_int;
type StrerrorFn = unsafe extern "C" fn(c_int, *mut c_char, usize) -> c_int;
type LogLevelFn = unsafe extern "C" fn(c_int);
type NewPacketFn = unsafe extern "C" fn(Raw, c_int) -> c_int;
type FindDecoderFn = unsafe extern "C" fn(*const c_char) -> *const c_void;
type AllocContextFn = unsafe extern "C" fn(*const c_void) -> Raw;
type OpenFn = unsafe extern "C" fn(Raw, *const c_void, *mut Raw) -> c_int;
type SendFn = unsafe extern "C" fn(Raw, *const c_void) -> c_int;
type ReceiveFn = unsafe extern "C" fn(Raw, Raw) -> c_int;

/// The loaded functions with the signatures of the pinned headers.
pub(super) struct Ffmpeg {
    pub frame_alloc: AllocFn,
    pub frame_free: FreeFn,
    pub frame_ref: RefFn,
    pub frame_unref: UnrefFn,
    pub buffer_unref: FreeFn,
    opt_set: OptSetFn,
    opt_set_int: OptSetIntFn,
    opt_get_int: OptGetIntFn,
    opt_find: OptFindFn,
    pub hwdevice_ctx_alloc: HwAllocFn,
    pub hwdevice_ctx_init: HwInitFn,
    pub hwdevice_ctx_create: HwCreateFn,
    pub hwframe_transfer_data: TransferFn,
    strerror: StrerrorFn,
    pub packet_alloc: AllocFn,
    pub packet_free: FreeFn,
    pub new_packet: NewPacketFn,
    pub packet_unref: UnrefFn,
    pub find_decoder_by_name: FindDecoderFn,
    pub alloc_context3: AllocContextFn,
    pub free_context: FreeFn,
    pub open2: OpenFn,
    pub send_packet: SendFn,
    pub receive_frame: ReceiveFn,
    pub formats: PixelFormats,
    /// `AV_HWDEVICE_TYPE_D3D11VA` and `AV_HWDEVICE_TYPE_DXVA2` of this build.
    pub d3d11va: c_int,
    pub dxva2: c_int,
    pub dir: PathBuf,
    /// `avutil_version()`, `avcodec_version()`.
    pub versions: (u32, u32),
    _libraries: Vec<Library>,
}

fn unsupported(text: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, text.into())
}

fn cstring(text: &str) -> io::Result<CString> {
    CString::new(text).map_err(|_| unsupported("FFmpeg name with a NUL byte"))
}

/// `T` from an entry of the symbol table.
///
/// # Safety
/// `T` must be the function pointer type of the symbol's C declaration.
unsafe fn typed<T: Copy>(table: &[AnyFn], symbol: Symbol) -> T {
    debug_assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<AnyFn>());
    // SAFETY: same size (function pointers); the caller names the type.
    unsafe { std::mem::transmute_copy::<AnyFn, T>(&table[symbol as usize]) }
}

fn load_from(dir: &Path) -> io::Result<Ffmpeg> {
    let mut libraries = Vec::with_capacity(DLLS.len());
    for name in DLLS {
        let path = dir.join(name);
        // SAFETY: the directory matched the pins (size and SHA-256 of every
        // DLL) right before; imports come from the DLL's own directory or
        // System32 only. The initialisers are FFmpeg's own.
        let library = unsafe {
            Library::load_with_flags(
                &path,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        }
        .map_err(|error| unsupported(format!("{}: {error}", path.display())))?;
        libraries.push(library);
    }
    let mut table: Vec<AnyFn> = Vec::with_capacity(Symbol::ALL.len());
    for symbol in Symbol::ALL {
        // SAFETY: only the address is taken here; `typed` gives it its type.
        let function =
            unsafe { libraries[symbol.library()].get::<AnyFn>(symbol.name().as_bytes()) }
                .map_err(|error| unsupported(format!("{}: {error}", symbol.name())))?;
        table.push(*function);
    }
    // SAFETY: each type below is the C declaration of its symbol in the
    // pinned headers (`ffmpeg_plan` module head).
    let ffmpeg = unsafe {
        let avutil_version: VersionFn = typed(&table, Symbol::AvutilVersion);
        let avcodec_version: VersionFn = typed(&table, Symbol::AvcodecVersion);
        let get_pix_fmt: ByNameFn = typed(&table, Symbol::GetPixFmt);
        let device_type: ByNameFn = typed(&table, Symbol::HwdeviceFindTypeByName);
        let log_level: LogLevelFn = typed(&table, Symbol::LogSetLevel);
        let versions = (avutil_version(), avcodec_version());
        if !plan::version_matches(versions.0, AVUTIL_MAJOR)
            || !plan::version_matches(versions.1, AVCODEC_MAJOR)
        {
            return Err(unsupported(format!(
                "FFmpeg avutil {} / avcodec {} instead of {AVUTIL_MAJOR} / {AVCODEC_MAJOR}",
                plan::version_text(versions.0),
                plan::version_text(versions.1)
            )));
        }
        let code = |name: &str| -> io::Result<c_int> {
            let name = cstring(name)?;
            Ok(get_pix_fmt(name.as_ptr()))
        };
        let [d3d11, dxva2, nv12, yuv420p] = PixelFormats::NAMES;
        let formats = PixelFormats {
            d3d11: code(d3d11)?,
            dxva2: code(dxva2)?,
            nv12: code(nv12)?,
            yuv420p: code(yuv420p)?,
        };
        let d3d11va = device_type(cstring("d3d11va")?.as_ptr());
        let dxva2 = device_type(cstring("dxva2")?.as_ptr());
        log_level(plan::AV_LOG_ERROR);
        Ffmpeg {
            frame_alloc: typed(&table, Symbol::FrameAlloc),
            frame_free: typed(&table, Symbol::FrameFree),
            frame_ref: typed(&table, Symbol::FrameRef),
            frame_unref: typed(&table, Symbol::FrameUnref),
            buffer_unref: typed(&table, Symbol::BufferUnref),
            opt_set: typed(&table, Symbol::OptSet),
            opt_set_int: typed(&table, Symbol::OptSetInt),
            opt_get_int: typed(&table, Symbol::OptGetInt),
            opt_find: typed(&table, Symbol::OptFind),
            hwdevice_ctx_alloc: typed(&table, Symbol::HwdeviceCtxAlloc),
            hwdevice_ctx_init: typed(&table, Symbol::HwdeviceCtxInit),
            hwdevice_ctx_create: typed(&table, Symbol::HwdeviceCtxCreate),
            hwframe_transfer_data: typed(&table, Symbol::HwframeTransferData),
            strerror: typed(&table, Symbol::Strerror),
            packet_alloc: typed(&table, Symbol::PacketAlloc),
            packet_free: typed(&table, Symbol::PacketFree),
            new_packet: typed(&table, Symbol::NewPacket),
            packet_unref: typed(&table, Symbol::PacketUnref),
            find_decoder_by_name: typed(&table, Symbol::FindDecoderByName),
            alloc_context3: typed(&table, Symbol::AllocContext3),
            free_context: typed(&table, Symbol::FreeContext),
            open2: typed(&table, Symbol::Open2),
            send_packet: typed(&table, Symbol::SendPacket),
            receive_frame: typed(&table, Symbol::ReceiveFrame),
            formats,
            d3d11va,
            dxva2,
            dir: dir.to_path_buf(),
            versions,
            _libraries: libraries,
        }
    };
    if !ffmpeg.formats.valid() {
        return Err(unsupported("FFmpeg lacks a pixel format the viewer needs"));
    }
    ffmpeg.check_layouts()?;
    Ok(ffmpeg)
}

impl Ffmpeg {
    /// The fingerprints of a fresh frame and packet (module head).
    fn check_layouts(&self) -> io::Result<()> {
        // SAFETY: the probes read at most `sizeof(AVFrame)` resp.
        // `sizeof(AVPacket)` bytes of objects FFmpeg just allocated (both
        // structs are larger than the probes in the pinned headers); both
        // are freed with their own functions.
        unsafe {
            let mut frame = (self.frame_alloc)();
            if frame.is_null() {
                return Err(unsupported("av_frame_alloc failed"));
            }
            let probe = std::slice::from_raw_parts(frame as *const u8, layout::FRAME_PROBE);
            let frame_ok = plan::fresh_frame_ok(probe, frame as u64);
            (self.frame_free)(&mut frame);
            let mut packet = (self.packet_alloc)();
            if packet.is_null() {
                return Err(unsupported("av_packet_alloc failed"));
            }
            let probe = std::slice::from_raw_parts(packet as *const u8, layout::PACKET_PROBE);
            let packet_ok = plan::fresh_packet_ok(probe);
            (self.packet_free)(&mut packet);
            if !frame_ok || !packet_ok {
                return Err(unsupported(
                    "the FFmpeg DLLs do not have the AVFrame/AVPacket layout of the pinned headers",
                ));
            }
        }
        Ok(())
    }

    /// FFmpeg's text of an error code.
    pub fn error_text(&self, code: c_int) -> String {
        let mut buffer = [0 as c_char; 128];
        // SAFETY: the buffer and its size are passed together; av_strerror
        // always terminates it.
        unsafe { (self.strerror)(code, buffer.as_mut_ptr(), buffer.len()) };
        // SAFETY: terminated above.
        let text = unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) };
        format!("{} ({code})", text.to_string_lossy())
    }

    /// `code` as a result: negative codes are errors named with `what`.
    pub fn check(&self, code: c_int, what: &str) -> io::Result<c_int> {
        if code < 0 {
            Err(io::Error::other(format!(
                "{what}: {}",
                self.error_text(code)
            )))
        } else {
            Ok(code)
        }
    }

    /// The byte offset of the option `name` of the object `object` (an
    /// `AVCodecContext`), from its class's option table.
    ///
    /// # Safety
    /// `object` must be a live object whose first field is its `AVClass`.
    pub unsafe fn option_offset(&self, object: Raw, name: &str) -> Option<usize> {
        let name = cstring(name).ok()?;
        // SAFETY: per the contract; the returned option lives in the DLL.
        unsafe {
            let option = (self.opt_find)(object, name.as_ptr(), std::ptr::null(), 0, 0);
            if option.is_null() {
                return None;
            }
            let offset = std::ptr::read_unaligned(
                (option as *const u8).add(layout::OPTION_OFFSET) as *const c_int
            );
            usize::try_from(offset).ok()
        }
    }

    /// Sets `options` on `object`; the first refusal is the error.
    ///
    /// # Safety
    /// As `option_offset`.
    pub unsafe fn set_options(
        &self,
        object: Raw,
        options: &[(&'static str, OptionValue)],
    ) -> io::Result<()> {
        for (name, value) in options {
            let key = cstring(name)?;
            // SAFETY: per the contract; the strings live through the call.
            let code = unsafe {
                match value {
                    OptionValue::Text(text) => {
                        let text = cstring(text)?;
                        (self.opt_set)(object, key.as_ptr(), text.as_ptr(), 0)
                    }
                    OptionValue::Int(number) => {
                        (self.opt_set_int)(object, key.as_ptr(), *number, 0)
                    }
                }
            };
            self.check(code, &format!("FFmpeg option {name}"))?;
        }
        Ok(())
    }

    /// An integer option of `object` (`colorspace`, `color_range`).
    ///
    /// # Safety
    /// As `option_offset`.
    pub unsafe fn get_int(&self, object: Raw, name: &str) -> Option<i64> {
        let key = cstring(name).ok()?;
        let mut value = 0i64;
        // SAFETY: per the contract; `value` lives through the call.
        let code = unsafe { (self.opt_get_int)(object, key.as_ptr(), 0, &mut value) };
        (code >= 0).then_some(value)
    }
}

enum State {
    Untried,
    Loaded(Arc<Ffmpeg>),
    /// Why it failed and when.
    Failed(String, Instant),
}

static STATE: Mutex<State> = Mutex::new(State::Untried);

/// A failed load is tried again after this long (fix of 2026-11-18: one
/// failure, e.g. while an update replaced the DLLs, left the main window's
/// viewer without FFmpeg for days; under „Dekodier-Bibliothek: FFmpeg“ only
/// the tiles were left).
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// FFmpeg of this process, or why it is missing. The first call checks the
/// DLLs against their pins (a few hundred milliseconds for 95 MB) and loads
/// them; a failure stays until `installed` reports a new copy or
/// `RETRY_AFTER` passed.
pub(super) fn ffmpeg() -> io::Result<Arc<Ffmpeg>> {
    let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    match &*state {
        State::Loaded(ffmpeg) => return Ok(ffmpeg.clone()),
        State::Failed(reason, at) if at.elapsed() < RETRY_AFTER => {
            return Err(unsupported(reason.clone()))
        }
        State::Failed(..) | State::Untried => {}
    }
    let loaded = super::ffmpeg_runtime::verified_dir()
        .map_err(unsupported)
        .and_then(|dir| load_from(&dir));
    match loaded {
        Ok(ffmpeg) => {
            eprintln!(
                "FFmpeg avutil {} / avcodec {} aus {}",
                plan::version_text(ffmpeg.versions.0),
                plan::version_text(ffmpeg.versions.1),
                ffmpeg.dir.display()
            );
            let ffmpeg = Arc::new(ffmpeg);
            *state = State::Loaded(ffmpeg.clone());
            Ok(ffmpeg)
        }
        Err(error) => {
            *state = State::Failed(error.to_string(), Instant::now());
            Err(error)
        }
    }
}

/// Whether FFmpeg loads in this process.
pub(super) fn present() -> bool {
    ffmpeg().is_ok()
}

/// A new copy of the DLLs was installed: the next call tries again.
pub(super) fn installed() {
    let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    if matches!(*state, State::Failed(..)) {
        *state = State::Untried;
    }
}
