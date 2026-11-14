//! The pointer as an overlay (plan 2026-11-14, package P6, decision C5).
//! A host that can take the pointer out of the picture (X11 through XFixes,
//! Windows through Graphics Capture without cursor capture) sends the
//! pointer's shape (on change) and position (on movement, at most once per
//! picture tick) as control frames; the viewer draws it over the picture, at
//! its own pointer position while it controls. The viewer asks for this with
//! `CURSOR_REQUEST` after the host announced `cursor_overlay` in its hello;
//! without the announcement the viewer never sends the frame, and a host
//! that cannot separate the pointer keeps it in the picture (v0.5.196).
//!
//! Three parts live here: the wire frames (`CURSOR_SHAPE`, `CURSOR_POSITION`
//! and the host process's `CURSOR` frame), the state inside the host process
//! (`WorkerCursor`, one per process because one process serves one session)
//! and the state of the daemon and the viewer (`CursorFeed`, `CursorView`).
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::pixel_convert::FrameSize;
use super::wire::{invalid, BeReader};
use super::wire_hello::{ScreenServerHello, SessionMode};

/// Version of the overlay contract, announced in `ScreenServerHello`.
pub const CURSOR_OVERLAY_V1: u16 = 1;
/// Viewer to host: payload `[1]` (overlay wanted) or `[0]`.
pub const CURSOR_REQUEST: u8 = 47;
/// Host to viewer: the pointer's shape.
pub const CURSOR_SHAPE: u8 = 48;
/// Host to viewer: the pointer's position.
pub const CURSOR_POSITION: u8 = 49;
/// Most pixels of a shape on the wire: 16 000 · 4 bytes plus the header stay
/// below the 64 KiB of a control frame.
pub const MAX_SHAPE_PIXELS: u64 = 16_000;
/// Largest pointer image of the platform the host accepts.
const MAX_SOURCE_SIDE: u32 = 512;
const SHAPE_HEADER: usize = 14;

/// Whether a viewer may send `CURSOR_REQUEST` on this connection.
pub fn negotiated(server: &ScreenServerHello, mode: SessionMode, metadata: bool) -> bool {
    server.cursor_overlay == Some(CURSOR_OVERLAY_V1)
        && !metadata
        && matches!(mode, SessionMode::View | SessionMode::Control)
}

/// The pointer's image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorShape {
    /// Names the shape in `CursorPosition::shape_id`.
    pub id: u32,
    pub width: u16,
    pub height: u16,
    pub hot_x: i16,
    pub hot_y: i16,
    /// Source pixels per shape pixel: a pointer above `MAX_SHAPE_PIXELS` is
    /// thinned out by this factor and drawn that much larger.
    pub shrink: u16,
    /// `width * height` pixels as RGBA bytes, colors premultiplied.
    pub rgba: Vec<u8>,
}

impl CursorShape {
    /// A shape from the pixels `pixel(x, y)` of a `width` x `height` image.
    pub fn sampled(
        id: u32,
        width: u32,
        height: u32,
        hot_x: i32,
        hot_y: i32,
        pixel: impl Fn(u32, u32) -> [u8; 4],
    ) -> Option<Self> {
        if width == 0 || height == 0 || width > MAX_SOURCE_SIDE || height > MAX_SOURCE_SIDE {
            return None;
        }
        let mut shrink = 1u32;
        while u64::from(width.div_ceil(shrink)) * u64::from(height.div_ceil(shrink))
            > MAX_SHAPE_PIXELS
        {
            shrink += 1;
        }
        let (columns, rows) = (width.div_ceil(shrink), height.div_ceil(shrink));
        let mut rgba = Vec::with_capacity((columns * rows * 4) as usize);
        for row in 0..rows {
            for column in 0..columns {
                let x = (column * shrink + shrink / 2).min(width - 1);
                let y = (row * shrink + shrink / 2).min(height - 1);
                rgba.extend_from_slice(&pixel(x, y));
            }
        }
        let hot = |value: i32| (value / shrink as i32).clamp(-32_768, 32_767) as i16;
        Some(Self {
            id,
            width: columns as u16,
            height: rows as u16,
            hot_x: hot(hot_x),
            hot_y: hot(hot_y),
            shrink: shrink as u16,
            rgba,
        })
    }

    /// A shape from `0xAARRGGBB` pixels with premultiplied colors (XFixes).
    pub fn from_argb(
        id: u32,
        width: u32,
        height: u32,
        hot_x: i32,
        hot_y: i32,
        argb: &[u32],
    ) -> Option<Self> {
        if (argb.len() as u64) < u64::from(width) * u64::from(height) {
            return None;
        }
        Self::sampled(id, width, height, hot_x, hot_y, |x, y| {
            let pixel = argb[(y * width + x) as usize];
            [
                (pixel >> 16) as u8,
                (pixel >> 8) as u8,
                pixel as u8,
                (pixel >> 24) as u8,
            ]
        })
    }

    fn validate(&self) -> io::Result<()> {
        let pixels = u64::from(self.width) * u64::from(self.height);
        if self.width == 0
            || self.height == 0
            || self.shrink == 0
            || pixels > MAX_SHAPE_PIXELS
            || self.rgba.len() as u64 != pixels * 4
        {
            return Err(invalid("pointer shape has an invalid size"));
        }
        Ok(())
    }

    pub fn encode(&self) -> io::Result<Vec<u8>> {
        self.validate()?;
        let mut out = Vec::with_capacity(SHAPE_HEADER + self.rgba.len());
        out.extend_from_slice(&self.id.to_be_bytes());
        out.extend_from_slice(&self.width.to_be_bytes());
        out.extend_from_slice(&self.height.to_be_bytes());
        out.extend_from_slice(&self.hot_x.to_be_bytes());
        out.extend_from_slice(&self.hot_y.to_be_bytes());
        out.extend_from_slice(&self.shrink.to_be_bytes());
        out.extend_from_slice(&self.rgba);
        Ok(out)
    }

    pub fn decode(payload: &[u8]) -> io::Result<Self> {
        let mut reader = BeReader::new(payload);
        let id = reader.u32()?;
        let width = reader.u16()?;
        let height = reader.u16()?;
        let hot_x = reader.i16()?;
        let hot_y = reader.i16()?;
        let shrink = reader.u16()?;
        let rest = reader.remaining();
        let rgba = reader.bytes(rest)?.to_vec();
        let shape = Self {
            id,
            width,
            height,
            hot_x,
            hot_y,
            shrink,
            rgba,
        };
        shape.validate()?;
        Ok(shape)
    }
}

/// Where the pointer is, in pixels of the captured area (not of the scaled
/// picture), measured at its hot spot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorPosition {
    /// The shape that belongs to this position.
    pub shape_id: u32,
    /// False while the pointer is hidden or outside the captured area.
    pub visible: bool,
    pub x: i32,
    pub y: i32,
    /// The captured area, so the viewer maps to its picture at any scale.
    pub area: FrameSize,
}

impl CursorPosition {
    /// No pointer to draw. The host sends it first after every
    /// `CURSOR_REQUEST` and every change of the session's source, so a viewer
    /// never keeps the pointer of the state before.
    pub const fn hidden() -> Self {
        Self {
            shape_id: 0,
            visible: false,
            x: 0,
            y: 0,
            area: FrameSize::new(1, 1),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(21);
        out.extend_from_slice(&self.shape_id.to_be_bytes());
        out.push(u8::from(self.visible));
        out.extend_from_slice(&self.x.to_be_bytes());
        out.extend_from_slice(&self.y.to_be_bytes());
        out.extend_from_slice(&self.area.width.to_be_bytes());
        out.extend_from_slice(&self.area.height.to_be_bytes());
        out
    }

    pub fn decode(payload: &[u8]) -> io::Result<Self> {
        let mut reader = BeReader::new(payload);
        let shape_id = reader.u32()?;
        let visible = match reader.u8()? {
            0 => false,
            1 => true,
            _ => return Err(invalid("pointer position has an invalid flag")),
        };
        let x = reader.u32()? as i32;
        let y = reader.u32()? as i32;
        let area = FrameSize::new(reader.u32()?, reader.u32()?);
        reader.finish()?;
        if area.width == 0 || area.height == 0 {
            return Err(invalid("pointer position names an empty area"));
        }
        Ok(Self {
            shape_id,
            visible,
            x,
            y,
            area,
        })
    }
}

/// One change of the pointer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CursorUpdate {
    Shape(CursorShape),
    Position(CursorPosition),
}

/// The payload of the host process's `CURSOR` frame.
pub fn encode_update(update: &CursorUpdate) -> io::Result<Vec<u8>> {
    let (kind, body) = match update {
        CursorUpdate::Shape(shape) => (0u8, shape.encode()?),
        CursorUpdate::Position(position) => (1u8, position.encode()),
    };
    let mut out = Vec::with_capacity(body.len() + 1);
    out.push(kind);
    out.extend_from_slice(&body);
    Ok(out)
}

pub fn decode_update(payload: &[u8]) -> io::Result<CursorUpdate> {
    let (kind, body) = payload
        .split_first()
        .ok_or_else(|| invalid("empty pointer update"))?;
    match kind {
        0 => Ok(CursorUpdate::Shape(CursorShape::decode(body)?)),
        1 => Ok(CursorUpdate::Position(CursorPosition::decode(body)?)),
        _ => Err(invalid("unknown pointer update")),
    }
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct WorkerState {
    /// The platform's identity of the current shape (X11 cursor serial,
    /// Windows cursor handle): the shape is read again only when it changes.
    serial: Option<u64>,
    shape: Option<CursorShape>,
    shape_unsent: bool,
    position: Option<CursorPosition>,
    position_unsent: bool,
    next_id: u32,
}

/// The pointer inside the host process: the capture publishes shape and
/// position, the process's request loop sends what changed after each
/// answer. Nothing is sent, and the capture keeps drawing the pointer into
/// the picture, until the daemon switched the overlay on. The daemon's wish
/// may come before the capture can separate the pointer (Windows creates its
/// capture session with the first picture): it is kept and takes effect then.
pub struct WorkerCursor {
    wanted: AtomicBool,
    separate: AtomicBool,
    supported: AtomicBool,
    state: Mutex<WorkerState>,
    /// The platform's switch (Windows: cursor capture of the capture session).
    hook: Mutex<Option<Box<dyn Fn(bool) + Send + Sync>>>,
}

static WORKER: WorkerCursor = WorkerCursor::new();

impl Default for WorkerCursor {
    fn default() -> Self {
        Self::new()
    }
}

/// The pointer of this host process.
pub fn worker() -> &'static WorkerCursor {
    &WORKER
}

impl WorkerCursor {
    pub const fn new() -> Self {
        Self {
            wanted: AtomicBool::new(false),
            separate: AtomicBool::new(false),
            supported: AtomicBool::new(false),
            state: Mutex::new(WorkerState {
                serial: None,
                shape: None,
                shape_unsent: false,
                position: None,
                position_unsent: false,
                next_id: 0,
            }),
            hook: Mutex::new(None),
        }
    }

    /// Whether the pointer stays out of the picture.
    pub fn separated(&self) -> bool {
        self.separate.load(Ordering::Acquire)
    }

    /// The capture of this process can read the pointer on its own; a wish
    /// the daemon sent before takes effect now.
    pub fn set_supported(&self) {
        self.supported.store(true, Ordering::Release);
        self.apply(false);
    }

    pub fn set_hook(&self, hook: Box<dyn Fn(bool) + Send + Sync>) {
        *locked(&self.hook) = Some(hook);
    }

    /// The daemon's wish for the overlay; the answer tells whether it is on
    /// now (`false` while the capture cannot separate the pointer: it keeps
    /// blending, and the wish stays for a capture that can). The next sends
    /// repeat shape and position, for a viewer that starts.
    pub fn set_separate(&self, on: bool) -> bool {
        self.wanted.store(on, Ordering::Release);
        self.apply(true)
    }

    /// Brings the overlay to what is wanted and possible. `repeat`: the
    /// viewer starts over, so shape and position go out again and the
    /// platform's switch hears the state even when it did not change.
    fn apply(&self, repeat: bool) -> bool {
        let on = self.wanted.load(Ordering::Acquire) && self.supported.load(Ordering::Acquire);
        let changed = repeat || on != self.separate.load(Ordering::Acquire);
        if changed {
            let mut state = locked(&self.state);
            state.shape_unsent = on && state.shape.is_some();
            state.position_unsent = on && state.position.is_some();
        }
        self.separate.store(on, Ordering::Release);
        if changed {
            if let Some(hook) = locked(&self.hook).as_ref() {
                hook(on);
            }
        }
        on
    }

    pub fn shape_serial(&self) -> Option<u64> {
        locked(&self.state).serial
    }

    pub fn next_shape_id(&self) -> u32 {
        let mut state = locked(&self.state);
        state.next_id = state.next_id.wrapping_add(1).max(1);
        state.next_id
    }

    /// The pointer has the shape `serial`; `None` when it cannot be sent
    /// (too large): the pointer is then hidden for the viewer.
    pub fn publish_shape(&self, serial: u64, shape: Option<CursorShape>) {
        let mut state = locked(&self.state);
        state.serial = Some(serial);
        state.shape_unsent = shape.is_some();
        state.shape = shape;
    }

    /// The pointer's hot spot is at `x`, `y` of `area`; `inside` is false
    /// when the pointer is hidden or outside the area.
    pub fn publish_position(&self, x: i32, y: i32, area: FrameSize, inside: bool) {
        let mut state = locked(&self.state);
        let shape_id = state.shape.as_ref().map(|shape| shape.id);
        let position = CursorPosition {
            shape_id: shape_id.unwrap_or(0),
            visible: inside && shape_id.is_some(),
            x,
            y,
            area,
        };
        if state.position != Some(position) {
            state.position = Some(position);
            state.position_unsent = true;
        }
    }

    pub fn has_pending(&self) -> bool {
        if !self.separated() {
            return false;
        }
        let state = locked(&self.state);
        state.shape_unsent || state.position_unsent
    }

    /// What changed since the last call: the shape first, then the position.
    pub fn take(&self) -> Vec<CursorUpdate> {
        if !self.separated() {
            return Vec::new();
        }
        let mut state = locked(&self.state);
        let mut updates = Vec::new();
        if std::mem::take(&mut state.shape_unsent) {
            if let Some(shape) = state.shape.clone() {
                updates.push(CursorUpdate::Shape(shape));
            }
        }
        if std::mem::take(&mut state.position_unsent) {
            if let Some(position) = state.position {
                updates.push(CursorUpdate::Position(position));
            }
        }
        updates
    }

    /// Writes what changed as `CURSOR` frames to the daemon.
    pub fn flush(&self, writer: &mut impl io::Write) -> io::Result<()> {
        for update in self.take() {
            super::worker_wire::write_message(
                &mut *writer,
                super::worker_wire::CURSOR,
                &encode_update(&update)?,
            )?;
        }
        Ok(())
    }
}

#[path = "cursor_feed.rs"]
mod feed;
pub use feed::{CursorFeed, CursorSeen, CursorView};
#[path = "cursor_mask.rs"]
mod mask;
pub use mask::from_two_backgrounds;

#[cfg(test)]
#[path = "cursor_overlay_tests.rs"]
mod tests;
