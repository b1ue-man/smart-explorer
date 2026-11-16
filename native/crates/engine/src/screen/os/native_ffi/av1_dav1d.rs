//! dav1d (`libdav1d.so.7`/`.6`, checked against 1.2.1 and 1.4.1 `dav1d/*.h`)
//! runtime wrapper for `av1_soft.rs`.
use std::ffi::{c_int, c_void};
use std::io;

use libloading::Library;

use super::{open, symbol, Handle};
use crate::screen::core::pixel_convert::{FrameSize, I420Image};
use crate::screen::core::wire::invalid;

/// `DAV1D_PIXEL_LAYOUT_I420` (headers.h); `DAV1D_ERR(EAGAIN)` = −EAGAIN.
const DAV1D_PIXEL_LAYOUT_I420: c_int = 1;
const DAV1D_EAGAIN: c_int = -libc::EAGAIN;

/// Whether any dav1d SONAME in `libraries` loads with `dav1d_open`.
pub(in crate::screen::os) fn dav1d_available(libraries: &[&str]) -> bool {
    libraries.iter().any(|name| {
        open(name)
            .and_then(|lib| symbol::<*const c_void>(&lib, name, "dav1d_open"))
            .is_ok()
    })
}

/// `Dav1dData` (72 bytes in dav1d 1.4) in a larger block.
#[repr(C)]
struct Dav1dData {
    data: *const u8,
    sz: usize,
    rest: [u64; 30],
}

/// `Dav1dPicture` (272 bytes in dav1d 1.4) in a larger block: the planes,
/// the strides and `Dav1dPictureParameters` lead it in every 1.x release.
#[repr(C)]
struct Dav1dPicture {
    seq_hdr: *mut c_void,
    frame_hdr: *mut c_void,
    data: [*mut u8; 3],
    stride: [isize; 2],
    w: c_int,
    h: c_int,
    layout: c_int,
    bpc: c_int,
    rest: [u64; 120],
}

/// One dav1d decoder; it gives I420 pictures.
pub(in crate::screen::os) struct Dav1dDecoder {
    context: Handle,
    send: unsafe extern "C" fn(Handle, *mut Dav1dData) -> c_int,
    get: unsafe extern "C" fn(Handle, *mut Dav1dPicture) -> c_int,
    data_create: unsafe extern "C" fn(*mut Dav1dData, usize) -> *mut u8,
    data_unref: unsafe extern "C" fn(*mut Dav1dData),
    picture_unref: unsafe extern "C" fn(*mut Dav1dPicture),
    close: unsafe extern "C" fn(*mut Handle),
    /// Planes of a shown picture given back (`recycle`, r1): the next
    /// picture is copied into them without a new allocation.
    spare: Option<I420Image>,
    _library: Library,
}

// SAFETY: one owner at a time; dav1d has no thread affinity for a context.
unsafe impl Send for Dav1dDecoder {}

impl Dav1dDecoder {
    /// Loads dav1d from the first of `libraries` and opens a decoder (`max_frame_delay` 1).
    pub(in crate::screen::os) fn open(libraries: &[&str]) -> io::Result<Self> {
        let mut found = Err(io::Error::new(io::ErrorKind::NotFound, "no dav1d library"));
        for name in libraries {
            found = open(name).map(|library| (library, *name));
            if found.is_ok() {
                break;
            }
        }
        let (lib, file) = found?;
        let defaults: unsafe extern "C" fn(*mut c_void) =
            symbol(&lib, file, "dav1d_default_settings")?;
        let open_context: unsafe extern "C" fn(*mut Handle, *const c_void) -> c_int =
            symbol(&lib, file, "dav1d_open")?;
        let send = symbol(&lib, file, "dav1d_send_data")?;
        let get = symbol(&lib, file, "dav1d_get_picture")?;
        let data_create = symbol(&lib, file, "dav1d_data_create")?;
        let data_unref = symbol(&lib, file, "dav1d_data_unref")?;
        let picture_unref = symbol(&lib, file, "dav1d_picture_unref")?;
        let close = symbol(&lib, file, "dav1d_close")?;
        let mut settings = [0u64; 64];
        // SAFETY: a zeroed block larger than `Dav1dSettings`.
        unsafe { defaults(settings.as_mut_ptr().cast()) };
        let leading = settings.as_mut_ptr().cast::<c_int>();
        // SAFETY: `n_threads` and `max_frame_delay` are the first two ints.
        unsafe {
            *leading = 0;
            *leading.add(1) = 1;
        }
        let mut context: Handle = std::ptr::null_mut();
        // SAFETY: all symbols are resolved before spawning dav1d threads.
        if unsafe { open_context(&mut context, settings.as_ptr().cast()) } != 0 || context.is_null()
        {
            return Err(io::Error::other("dav1d_open failed"));
        }
        Ok(Self {
            context,
            send,
            get,
            data_create,
            data_unref,
            picture_unref,
            close,
            spare: None,
            _library: lib,
        })
    }

    pub(in crate::screen::os) fn decode(&mut self, unit: &[u8]) -> io::Result<Option<I420Image>> {
        self.decode_expected(unit, None)
    }

    /// Decodes one temporal unit; `expected` bounds plane reservation before copy.
    pub(in crate::screen::os) fn decode_expected(
        &mut self,
        unit: &[u8],
        expected: Option<FrameSize>,
    ) -> io::Result<Option<I420Image>> {
        // SAFETY: plain C data, zero is its empty state.
        let mut data: Dav1dData = unsafe { std::mem::zeroed() };
        // SAFETY: dav1d allocates `unit.len()` bytes owned by `data`.
        let target = unsafe { (self.data_create)(&mut data, unit.len()) };
        if target.is_null() {
            return Err(io::Error::other("dav1d_data_create failed"));
        }
        // SAFETY: `target` has room for `unit.len()` bytes.
        unsafe { std::ptr::copy_nonoverlapping(unit.as_ptr(), target, unit.len()) };
        let mut sent = DAV1D_EAGAIN;
        for _ in 0..8 {
            // SAFETY: valid context and data; on EAGAIN `data` stays unconsumed.
            sent = unsafe { (self.send)(self.context, &mut data) };
            if sent != DAV1D_EAGAIN {
                break;
            }
            // SAFETY: drain one buffered picture so `dav1d_send_data` can accept `data`.
            let mut stale: Dav1dPicture = unsafe { std::mem::zeroed() };
            if unsafe { (self.get)(self.context, &mut stale) } == 0 {
                unsafe { (self.picture_unref)(&mut stale) };
            } else {
                break;
            }
        }
        if data.sz != 0 {
            // SAFETY: unconsumed data after an error is released here.
            unsafe { (self.data_unref)(&mut data) };
        }
        if sent != 0 {
            return Err(io::Error::other(format!("dav1d_send_data: {sent}")));
        }
        // SAFETY: plain C data, zero is its empty state.
        let mut picture: Dav1dPicture = unsafe { std::mem::zeroed() };
        // SAFETY: valid context; `picture` is written by the decoder.
        let got = unsafe { (self.get)(self.context, &mut picture) };
        if got == DAV1D_EAGAIN {
            return Ok(None);
        }
        if got != 0 {
            return Err(io::Error::other(format!("dav1d_get_picture: {got}")));
        }
        let result = copy_picture(&picture, expected, self.spare.take());
        // SAFETY: the picture came from `dav1d_get_picture`.
        unsafe { (self.picture_unref)(&mut picture) };
        result.map(Some)
    }

    /// Gives the planes of a shown picture back for the next one (r1).
    pub(in crate::screen::os) fn recycle(&mut self, planes: I420Image) {
        self.spare = Some(planes);
    }
}

fn copy_picture(
    picture: &Dav1dPicture,
    expected: Option<FrameSize>,
    reuse: Option<I420Image>,
) -> io::Result<I420Image> {
    if picture.layout != DAV1D_PIXEL_LAYOUT_I420
        || picture.bpc != 8
        || picture.w <= 0
        || picture.h <= 0
    {
        return Err(invalid("dav1d picture is not 8-bit 4:2:0"));
    }
    let size = FrameSize::new(picture.w as u32, picture.h as u32);
    if expected.is_some_and(|want| want != size) {
        return Err(invalid("dav1d picture differs from its picture group"));
    }
    let chroma = size.chroma();
    let (y, u, v) = reuse.map_or_else(Default::default, |old| (old.y, old.u, old.v));
    let plane = |index: usize,
                 stride: isize,
                 width: u32,
                 rows: u32,
                 mut out: Vec<u8>|
     -> io::Result<Vec<u8>> {
        let (width, rows) = (width as usize, rows as usize);
        let total = width
            .checked_mul(rows)
            .ok_or_else(|| invalid("dav1d plane size overflow"))?;
        if picture.data[index].is_null() || stride < width as isize {
            return Err(invalid("dav1d plane smaller than its picture"));
        }
        out.clear();
        out.reserve(total);
        for row in 0..rows {
            let offset = (row as isize)
                .checked_mul(stride)
                .ok_or_else(|| invalid("dav1d plane stride overflow"))?;
            // SAFETY: dav1d's plane has `rows` rows of `stride` bytes.
            let line =
                unsafe { std::slice::from_raw_parts(picture.data[index].offset(offset), width) };
            out.extend_from_slice(line);
        }
        Ok(out)
    };
    Ok(I420Image {
        size,
        y: plane(0, picture.stride[0], size.width, size.height, y)?,
        u: plane(1, picture.stride[1], chroma.width, chroma.height, u)?,
        v: plane(2, picture.stride[1], chroma.width, chroma.height, v)?,
    })
}

impl Drop for Dav1dDecoder {
    fn drop(&mut self) {
        // SAFETY: the context came from `dav1d_open`.
        unsafe { (self.close)(&mut self.context) };
    }
}
