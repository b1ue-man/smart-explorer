//! Software AV1 for Linux (BK4/BK6, B-25, B-38): SVT-AV1 (`libSvtAv1Enc.so.1`,
//! v1.5–1.x, checked against v1.7.0 `EbSvtAv1*.h` and `EbEncSettings.c`) and
//! dav1d (`libdav1d.so.7`/`.6`, checked against 1.2.1 and 1.4.1 `dav1d/*.h`),
//! loaded at run time by SONAME. Only leading ABI-stable fields are read inside
//! oversized zeroed blocks; SVT-AV1 configuration is set by parameter name.
use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::io;
use std::time::{Duration, Instant};

use libloading::Library;

use crate::screen::core::pixel_convert::{FrameSize, I420Image};
use crate::screen::core::wire::invalid;

pub(super) const SVT_LIBRARY: &str = "libSvtAv1Enc.so.1";
pub(super) const DAV1D_LIBRARIES: [&str; 2] = ["libdav1d.so.7", "libdav1d.so.6"];
/// `EB_ErrorNone`, `EB_NoErrorEmptyQueue` (EbSvtAv1.h).
const EB_ERROR_NONE: c_int = 0;
const EB_NO_ERROR_EMPTY_QUEUE: c_int = 0x8000_2033_u32 as c_int;
/// `EB_AV1_KEY_PICTURE`, `EB_AV1_INVALID_PICTURE` (EbSvtAv1.h).
const EB_AV1_KEY_PICTURE: u32 = 3;
const EB_AV1_INVALID_PICTURE: u32 = 0xff;
/// `EB_YUV420` (EbSvtAv1Formats.h), `EB_EIGHT_BIT`.
const EB_YUV420: u32 = 1;
const EB_EIGHT_BIT: u32 = 8;
/// How long one picture may take in the SVT-AV1 pipeline.
const SVT_DEADLINE: Duration = Duration::from_secs(2);

#[path = "av1_dav1d.rs"]
mod dav1d;
pub(super) use dav1d::{dav1d_available, Dav1dDecoder};

/// `EbBufferHeaderType` of SVT-AV1 1.7 (136 bytes) in a larger block.
#[repr(C)]
struct SvtBufferHeader {
    size: u32,
    p_buffer: *mut u8,
    n_filled_len: u32,
    n_alloc_len: u32,
    p_app_private: *mut c_void,
    wrapper_ptr: *mut c_void,
    n_tick_count: u32,
    dts: i64,
    pts: i64,
    qp: u32,
    pic_type: u32,
    rest: [u64; 24],
}

/// `EbSvtIOFormat` of SVT-AV1 ≥ 1.5 (64 bytes).
#[repr(C)]
struct SvtIoFormat {
    luma: *mut u8,
    cb: *mut u8,
    cr: *mut u8,
    y_stride: u32,
    cr_stride: u32,
    cb_stride: u32,
    width: u32,
    height: u32,
    org_x: u32,
    org_y: u32,
    color_fmt: u32,
    bit_depth: u32,
}

/// Screen settings: preset 12, CBR (`str_to_rc_mode` selects `PRED_LOW_DELAY_B`;
/// SVT-AV1 1.7 `svt_av1_verify_settings` forbids `enable-force-key-frames` in
/// CBR low-delay mode, so mid-stream key frames reopen the encoder).
pub(super) fn svt_settings(
    size: FrameSize,
    fps: u32,
    bits_per_second: u64,
) -> Vec<(&'static str, String)> {
    vec![
        ("width", size.width.to_string()),
        ("height", size.height.to_string()),
        ("fps-num", fps.max(1).to_string()),
        ("fps-denom", "1".into()),
        ("input-depth", "8".into()),
        ("preset", "12".into()),
        ("rc", "cbr".into()),
        (
            "tbr",
            format!("{}b", bits_per_second.clamp(1000, 100_000_000)),
        ),
        ("lookahead", "0".into()),
        ("scd", "0".into()),
        ("enable-tf", "0".into()),
        ("keyint", "-1".into()),
        ("color-primaries", "1".into()),
        ("transfer-characteristics", "1".into()),
        ("matrix-coefficients", "1".into()),
        ("color-range", "0".into()),
    ]
}

/// Whether `version` (`svt_av1_get_version`, "v1.7.0") has the 1.5+ layout.
pub(super) fn svt_version_supported(version: &str) -> bool {
    let mut parts = version
        .trim_start_matches('v')
        .split('.')
        .map(|part| part.parse::<u32>());
    matches!((parts.next(), parts.next()), (Some(Ok(1)), Some(Ok(minor))) if minor >= 5)
}

/// Whether `library` loads with a supported 1.5+ SVT-AV1 ABI.
pub(super) fn svt_available(library: &str) -> bool {
    let Ok(lib) = open(library) else {
        return false;
    };
    let Ok(version) =
        symbol::<unsafe extern "C" fn() -> *const c_char>(&lib, library, "svt_av1_get_version")
    else {
        return false;
    };
    // SAFETY: `svt_av1_get_version` returns a static NUL-terminated string.
    let ptr = unsafe { version() };
    !ptr.is_null()
        && svt_version_supported(&unsafe { CStr::from_ptr(ptr) }.to_string_lossy())
        && symbol::<*const c_void>(&lib, library, "svt_av1_enc_init_handle").is_ok()
}

fn symbol<T: Copy>(library: &Library, file: &str, name: &str) -> io::Result<T> {
    let mut bytes = name.as_bytes().to_vec();
    bytes.push(0);
    // SAFETY: `T` is the C signature of the named function as declared in
    // the headers listed above (spelled out at each call site).
    let found = unsafe { library.get::<T>(&bytes) };
    found
        .map(|symbol| *symbol)
        .map_err(|error| io::Error::other(format!("{file}: {name}: {error}")))
}

fn open(name: &str) -> io::Result<Library> {
    // SAFETY: loading the codec library runs only its own initializers.
    unsafe { Library::new(name) }
        .map_err(|error| io::Error::new(io::ErrorKind::NotFound, format!("{name}: {error}")))
}

type Handle = *mut c_void;

/// One SVT-AV1 encoder of a fixed size and rate.
pub(super) struct SvtAv1Encoder {
    handle: Handle,
    config_block: Vec<u64>,
    send: unsafe extern "C" fn(Handle, *mut SvtBufferHeader) -> c_int,
    get: unsafe extern "C" fn(Handle, *mut *mut SvtBufferHeader, u8) -> c_int,
    release: unsafe extern "C" fn(*mut *mut SvtBufferHeader),
    deinit: unsafe extern "C" fn(Handle) -> c_int,
    deinit_handle: unsafe extern "C" fn(Handle) -> c_int,
    library_name: String,
    size: FrameSize,
    fps: u32,
    bits_per_second: u64,
    frames: i64,
    initialized: bool,
    _library: Library,
}

// SAFETY: one owner at a time; SVT-AV1 has no caller thread affinity.
unsafe impl Send for SvtAv1Encoder {}

impl SvtAv1Encoder {
    /// Loads SVT-AV1 from `library` and opens an encoder.
    pub(super) fn open(
        library: &str,
        size: FrameSize,
        fps: u32,
        bits_per_second: u64,
    ) -> io::Result<Self> {
        let lib = open(library)?;
        let version: unsafe extern "C" fn() -> *const c_char =
            symbol(&lib, library, "svt_av1_get_version")?;
        // SAFETY: a static NUL-terminated version string of the library.
        let version_ptr = unsafe { version() };
        let text = if version_ptr.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(version_ptr) }
                .to_string_lossy()
                .into_owned()
        };
        if !svt_version_supported(&text) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("SVT-AV1 {text} is older than 1.5"),
            ));
        }
        let init_handle: unsafe extern "C" fn(*mut Handle, *mut c_void, *mut c_void) -> c_int =
            symbol(&lib, library, "svt_av1_enc_init_handle")?;
        let parse: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int =
            symbol(&lib, library, "svt_av1_enc_parse_parameter")?;
        let set: unsafe extern "C" fn(Handle, *mut c_void) -> c_int =
            symbol(&lib, library, "svt_av1_enc_set_parameter")?;
        let init: unsafe extern "C" fn(Handle) -> c_int =
            symbol(&lib, library, "svt_av1_enc_init")?;
        let mut encoder = Self {
            handle: std::ptr::null_mut(),
            config_block: vec![0u64; 8192],
            send: symbol(&lib, library, "svt_av1_enc_send_picture")?,
            get: symbol(&lib, library, "svt_av1_enc_get_packet")?,
            release: symbol(&lib, library, "svt_av1_enc_release_out_buffer")?,
            deinit: symbol(&lib, library, "svt_av1_enc_deinit")?,
            deinit_handle: symbol(&lib, library, "svt_av1_enc_deinit_handle")?,
            library_name: library.to_owned(),
            size,
            fps,
            bits_per_second,
            frames: 0,
            initialized: false,
            _library: lib,
        };
        let config = encoder.config_block.as_mut_ptr().cast::<c_void>();
        // SAFETY: the library fills the zeroed configuration block.
        if unsafe { init_handle(&mut encoder.handle, std::ptr::null_mut(), config) }
            != EB_ERROR_NONE
        {
            encoder.handle = std::ptr::null_mut();
            return Err(io::Error::other("svt_av1_enc_init_handle failed"));
        }
        for (name, value) in svt_settings(size, fps, bits_per_second) {
            let (name_c, value_c) = (CString::new(name)?, CString::new(value.clone())?);
            // SAFETY: valid configuration and NUL-terminated strings.
            if unsafe { parse(config, name_c.as_ptr(), value_c.as_ptr()) } != EB_ERROR_NONE {
                return Err(io::Error::other(format!("SVT-AV1 refused {name}={value}")));
            }
        }
        // SAFETY: valid handle and configuration.
        if unsafe { set(encoder.handle, config) } != EB_ERROR_NONE
            || unsafe { init(encoder.handle) } != EB_ERROR_NONE
        {
            return Err(io::Error::other("SVT-AV1 refused the screen settings"));
        }
        encoder.initialized = true;
        Ok(encoder)
    }

    /// Encodes one I420 picture and hands the borrowed packet slice to `deliver`.
    pub(super) fn encode_with(
        &mut self,
        yuv: &I420Image,
        keyframe: bool,
        deliver: &mut dyn FnMut(&[&[u8]]) -> io::Result<()>,
    ) -> io::Result<()> {
        if keyframe && self.frames > 0 {
            let lib = self.library_name.clone();
            *self = Self::open(&lib, self.size, self.fps, self.bits_per_second)?;
        }
        let size = self.size;
        let chroma = size.chroma();
        if yuv.size != size
            || yuv.y.len() < size.pixels() as usize
            || yuv.u.len() < chroma.pixels() as usize
            || yuv.v.len() < chroma.pixels() as usize
        {
            return Err(invalid("I420 picture differs from the SVT-AV1 encoder"));
        }
        let mut io_format = SvtIoFormat {
            luma: yuv.y.as_ptr().cast_mut(),
            cb: yuv.u.as_ptr().cast_mut(),
            cr: yuv.v.as_ptr().cast_mut(),
            y_stride: size.width,
            cr_stride: chroma.width,
            cb_stride: chroma.width,
            width: size.width,
            height: size.height,
            org_x: 0,
            org_y: 0,
            color_fmt: EB_YUV420,
            bit_depth: EB_EIGHT_BIT,
        };
        // SAFETY: plain C data; all-zero is the documented default header.
        let mut input: SvtBufferHeader = unsafe { std::mem::zeroed() };
        input.size = std::mem::size_of::<SvtBufferHeader>() as u32;
        input.p_buffer = (&mut io_format as *mut SvtIoFormat).cast();
        input.n_filled_len = (yuv.y.len() + yuv.u.len() + yuv.v.len()) as u32;
        input.pts = self.frames;
        input.pic_type = if keyframe {
            EB_AV1_KEY_PICTURE
        } else {
            EB_AV1_INVALID_PICTURE
        };
        // SAFETY: valid handle; SVT-AV1 copies the planes before returning.
        if unsafe { (self.send)(self.handle, &mut input) } != EB_ERROR_NONE {
            return Err(io::Error::other("svt_av1_enc_send_picture failed"));
        }
        self.frames += 1;
        let deadline = Instant::now() + SVT_DEADLINE;
        loop {
            let mut packet: *mut SvtBufferHeader = std::ptr::null_mut();
            // SAFETY: valid handle and writable packet slot (non-blocking).
            let result = unsafe { (self.get)(self.handle, &mut packet, 0) };
            if result == EB_ERROR_NONE && !packet.is_null() {
                // SAFETY: the library's packet stays valid until released.
                let header = unsafe { &*packet };
                let slice: &[u8] = if header.p_buffer.is_null() {
                    &[]
                } else {
                    // SAFETY: `n_filled_len` bytes of the packet buffer.
                    unsafe {
                        std::slice::from_raw_parts(header.p_buffer, header.n_filled_len as usize)
                    }
                };
                let delivered = deliver(&[slice]);
                // SAFETY: the packet came from `svt_av1_enc_get_packet`.
                unsafe { (self.release)(&mut packet) };
                return delivered;
            }
            if result != EB_NO_ERROR_EMPTY_QUEUE || Instant::now() >= deadline {
                return Err(io::Error::other("SVT-AV1 gave no packet for the picture"));
            }
            std::thread::sleep(Duration::from_micros(250));
        }
    }

    /// Encodes one I420 picture; the temporal unit (OBUs) of that picture.
    pub(super) fn encode(&mut self, yuv: &I420Image, keyframe: bool) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        self.encode_with(yuv, keyframe, &mut |parts| {
            for part in parts {
                out.extend_from_slice(part);
            }
            Ok(())
        })?;
        Ok(out)
    }
}

impl Drop for SvtAv1Encoder {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: the handle came from `svt_av1_enc_init_handle`; the
            // pipeline is torn down only when `svt_av1_enc_init` built it.
            unsafe {
                if self.initialized {
                    (self.deinit)(self.handle);
                }
                (self.deinit_handle)(self.handle);
            }
        }
    }
}

#[cfg(test)]
#[path = "bildkette_av1_task_tests.rs"]
mod bildkette_av1_task_tests;
