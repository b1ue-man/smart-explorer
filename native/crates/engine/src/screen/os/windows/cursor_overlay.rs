//! The pointer of a Windows host for the overlay (plan 2026-11-14, package
//! P6, decision C5). Graphics Capture draws the pointer into every picture
//! (`SetIsCursorCaptureEnabled`); once the viewer asked to draw it itself
//! (`screen::core::cursor_overlay`), the capture session stops drawing it and
//! a sampler thread of this host process reads shape (`GetCursorInfo`,
//! `GetIconInfo`, `DrawIconEx` onto black and white, which also recovers the
//! alpha of mask pointers; `cursor_mask.rs`) and position and publishes them
//! to the overlay. The position is the hot spot relative to the captured
//! source's rectangle (`input_send::source_area`); outside it, or while the
//! pointer is hidden, the viewer hides its overlay. Windows before 10 2004
//! cannot switch the pointer capture of a session: there the pointer stays
//! in every picture and this host never sends it beside.
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use windows::Graphics::Capture::GraphicsCaptureSession;

use super::{input_send, monitors};
use crate::screen::core::cursor_overlay::{
    from_two_backgrounds, worker, CursorShape, WorkerCursor,
};
use crate::screen::core::pixel_convert::FrameSize;

type RawHandle = *mut c_void;

/// Time between two readings of the pointer.
const SAMPLE_EVERY: Duration = Duration::from_millis(8);
/// `CURSORINFO::flags`: the pointer is shown.
const CURSOR_SHOWING: u32 = 1;
/// `DrawIconEx` flags `DI_NORMAL`: image and mask.
const DI_NORMAL: u32 = 3;
/// Largest side of a pointer image that is read (the platform has none above).
const MAX_SIDE: i32 = 512;

#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
struct CursorInfo {
    size: u32,
    flags: u32,
    cursor: RawHandle,
    position: Point,
}

#[repr(C)]
struct IconInfo {
    is_icon: i32,
    hot_x: u32,
    hot_y: u32,
    mask: RawHandle,
    color: RawHandle,
}

#[repr(C)]
struct BitmapInfo {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    image_size: u32,
    x_pixels: i32,
    y_pixels: i32,
    colors_used: u32,
    colors_important: u32,
    color: u32,
}

#[repr(C)]
struct BitmapFacts {
    kind: i32,
    width: i32,
    height: i32,
    stride: i32,
    planes: u16,
    bits: u16,
    data: RawHandle,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn GetCursorInfo(info: *mut CursorInfo) -> i32;
    fn GetIconInfo(icon: RawHandle, info: *mut IconInfo) -> i32;
    fn DrawIconEx(
        dc: RawHandle,
        x: i32,
        y: i32,
        icon: RawHandle,
        width: i32,
        height: i32,
        step: u32,
        brush: RawHandle,
        flags: u32,
    ) -> i32;
}
#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreateCompatibleDC(dc: RawHandle) -> RawHandle;
    fn CreateDIBSection(
        dc: RawHandle,
        info: *const BitmapInfo,
        usage: u32,
        bits: *mut RawHandle,
        section: RawHandle,
        offset: u32,
    ) -> RawHandle;
    fn SelectObject(dc: RawHandle, object: RawHandle) -> RawHandle;
    fn DeleteObject(object: RawHandle) -> i32;
    fn DeleteDC(dc: RawHandle) -> i32;
    fn GdiFlush() -> i32;
    fn GetObjectW(object: RawHandle, size: i32, data: *mut c_void) -> i32;
}

/// The pointer's size in pixels and its hot spot.
fn measure(cursor: RawHandle) -> Option<(i32, i32, i32, i32)> {
    let mut info = IconInfo {
        is_icon: 0,
        hot_x: 0,
        hot_y: 0,
        mask: std::ptr::null_mut(),
        color: std::ptr::null_mut(),
    };
    // SAFETY: a plain query of a pointer handle into a local of the size
    // `ICONINFO` has; the two bitmaps it returns are deleted below.
    if unsafe { GetIconInfo(cursor, &mut info) } == 0 {
        return None;
    }
    let mut facts = BitmapFacts {
        kind: 0,
        width: 0,
        height: 0,
        stride: 0,
        planes: 0,
        bits: 0,
        data: std::ptr::null_mut(),
    };
    // A monochrome pointer has only the mask, which holds AND and XOR halves.
    let (bitmap, parts) = if info.color.is_null() {
        (info.mask, 2)
    } else {
        (info.color, 1)
    };
    // SAFETY: `facts` has the layout and size of `BITMAP`; the bitmaps of
    // `GetIconInfo` belong to this caller and are deleted exactly once.
    let read = unsafe {
        let read = GetObjectW(
            bitmap,
            std::mem::size_of::<BitmapFacts>() as i32,
            (&mut facts as *mut BitmapFacts).cast(),
        );
        if !info.mask.is_null() {
            DeleteObject(info.mask);
        }
        if !info.color.is_null() {
            DeleteObject(info.color);
        }
        read
    };
    let (width, height) = (facts.width, facts.height / parts);
    (read != 0 && (1..=MAX_SIDE).contains(&width) && (1..=MAX_SIDE).contains(&height)).then_some((
        width,
        height,
        info.hot_x as i32,
        info.hot_y as i32,
    ))
}

/// The pointer drawn onto a `background` of one value (BGRA bytes).
fn render_on(cursor: RawHandle, width: i32, height: i32, background: u8) -> Option<Vec<u8>> {
    let info = BitmapInfo {
        size: 40,
        width,
        height: -height,
        planes: 1,
        bit_count: 32,
        compression: 0,
        image_size: 0,
        x_pixels: 0,
        y_pixels: 0,
        colors_used: 0,
        colors_important: 0,
        color: 0,
    };
    let bytes = width as usize * height as usize * 4;
    // SAFETY: the DIB section has the size named by `info`, so the `bytes`
    // read below stay inside it; every GDI object made here is released on
    // every path.
    unsafe {
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            return None;
        }
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(dc, &info, 0, &mut bits, std::ptr::null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            DeleteDC(dc);
            return None;
        }
        let previous = SelectObject(dc, bitmap);
        if previous.is_null() || previous as isize == -1 {
            DeleteObject(bitmap);
            DeleteDC(dc);
            return None;
        }
        let pixels = std::slice::from_raw_parts_mut(bits as *mut u8, bytes);
        pixels.fill(background);
        let drawn = DrawIconEx(
            dc,
            0,
            0,
            cursor,
            width,
            height,
            0,
            std::ptr::null_mut(),
            DI_NORMAL,
        );
        GdiFlush();
        let copied = (drawn != 0).then(|| pixels.to_vec());
        SelectObject(dc, previous);
        DeleteObject(bitmap);
        DeleteDC(dc);
        copied
    }
}

/// RGBA bytes with premultiplied colors of the pointer, from its renderings
/// onto black and onto white (`cursor_mask.rs`: alpha of mask pointers and of
/// pointers with an alpha channel alike, inverting pixels made visible).
fn render(cursor: RawHandle, width: i32, height: i32) -> Option<Vec<u8>> {
    let black = render_on(cursor, width, height, 0)?;
    let white = render_on(cursor, width, height, 255)?;
    Some(from_two_backgrounds(&black, &white, width as usize))
}

/// One reading of the pointer: the shape when its handle changed, the position
/// at every reading (the overlay keeps only what differs).
fn sample(overlay: &WorkerCursor, last_handle: &mut usize) {
    let mut info = CursorInfo {
        size: std::mem::size_of::<CursorInfo>() as u32,
        flags: 0,
        cursor: std::ptr::null_mut(),
        position: Point { x: 0, y: 0 },
    };
    // SAFETY: a plain query into a local whose `size` field is set.
    if unsafe { GetCursorInfo(&mut info) } == 0 {
        return;
    }
    let shown = info.flags & CURSOR_SHOWING != 0 && !info.cursor.is_null();
    let handle = info.cursor as usize;
    if shown && (handle != *last_handle || overlay.shape_serial().is_none()) {
        *last_handle = handle;
        let shape = measure(info.cursor).and_then(|(width, height, hot_x, hot_y)| {
            let rgba = render(info.cursor, width, height)?;
            CursorShape::sampled(
                overlay.next_shape_id(),
                width as u32,
                height as u32,
                hot_x,
                hot_y,
                |x, y| {
                    let at = ((y * width as u32 + x) * 4) as usize;
                    [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]]
                },
            )
        });
        overlay.publish_shape(handle as u64, shape);
    }
    let Some(area) = input_send::source_area() else {
        return;
    };
    let x = info.position.x - area.left;
    let y = info.position.y - area.top;
    let inside = shown && x >= 0 && y >= 0 && x < area.width && y < area.height;
    overlay.publish_position(
        x,
        y,
        FrameSize::new(area.width as u32, area.height as u32),
        inside,
    );
}

static SAMPLING: AtomicBool = AtomicBool::new(false);
/// The overlay switched the pointer capture of a session that already
/// delivers pictures: those so far show the pointer as it was before.
static SWITCHED: AtomicBool = AtomicBool::new(false);

/// Whether the pointer capture of the running session was switched since
/// the last call (`capture_wgc.rs` then makes sure a picture in the new
/// state follows, also on a screen that stands still).
pub(super) fn take_switched() -> bool {
    SWITCHED.swap(false, Ordering::AcqRel)
}

/// Starts the sampler thread unless it runs; it ends when the overlay is off.
fn start_sampler() {
    if SAMPLING.swap(true, Ordering::AcqRel) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("screen-cursor".into())
        .spawn(|| {
            monitors::ensure_dpi_aware();
            let overlay = worker();
            let mut last_handle = 0usize;
            while overlay.separated() {
                sample(overlay, &mut last_handle);
                std::thread::sleep(SAMPLE_EVERY);
            }
            SAMPLING.store(false, Ordering::Release);
            // Switched on again between the last check and the store above.
            if overlay.separated() {
                start_sampler();
            }
        });
    if spawned.is_err() {
        SAMPLING.store(false, Ordering::Release);
    }
}

/// Sets a new capture session's pointer capture to the overlay's state and
/// hands the session's switch to the overlay. A session that cannot switch
/// (Windows before 10 2004 has no `IGraphicsCaptureSession2`) keeps drawing
/// the pointer, and the overlay is not offered: the viewer never gets a
/// second pointer beside the one in the picture. A wish for the overlay that
/// came before this session existed takes effect here, through the switch.
pub(super) fn attach(session: &GraphicsCaptureSession) {
    let overlay = worker();
    if session
        .SetIsCursorCaptureEnabled(!overlay.separated())
        .is_err()
    {
        return;
    }
    let held = Mutex::new(session.clone());
    // Whether this session draws the pointer into its pictures.
    let drawn = AtomicBool::new(!overlay.separated());
    overlay.set_hook(Box::new(move |on| {
        if let Ok(session) = held.lock() {
            // A closed session answers with an error; the next one is attached.
            let switched = session.SetIsCursorCaptureEnabled(!on).is_ok();
            if switched && drawn.swap(!on, Ordering::AcqRel) == on {
                SWITCHED.store(true, Ordering::Release);
            }
        }
        if on {
            start_sampler();
        }
    }));
    overlay.set_supported();
    // This session has delivered no picture yet: what was switched until
    // here needs no picture to follow.
    SWITCHED.store(false, Ordering::Release);
    if overlay.separated() {
        start_sampler();
    }
}
