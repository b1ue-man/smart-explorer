//! Screen capture on X11 (SCR-M5, U19-6): a monitor or a window of the
//! session of `DISPLAY`, read with MIT-SHM into a memfd that the server
//! shares (`attach_fd`), or with plain `GetImage` when the server has no
//! usable MIT-SHM (the limit `x11_no_shm` says so). Windows come from their
//! composite pixmap when a compositor runs, else from the root window at the
//! window's place (overlapping windows then show: `x11_window_overlaps_
//! without_compositor`). The pointer is blended in from XFixes. DAMAGE is not
//! used (compositors hide the root's damage); a picture equal to the last one
//! by a 64-bit FNV-1a hash is `Unchanged`.
use std::ffi::c_void;
use std::io;
use std::os::fd::{FromRawFd, OwnedFd};

use x11rb::connection::Connection;
use x11rb::errors::ReplyError;
use x11rb::protocol::composite::ConnectionExt as _;
use x11rb::protocol::shm::ConnectionExt as _;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, ImageFormat, ImageOrder, MapState, Window,
};
use x11rb::protocol::ErrorKind;
use x11rb::rust_connection::RustConnection;

use super::x11_cursor;
use super::x11_monitors;
use super::x11_windows::{self, Atoms};
use crate::screen::core::host_backend::{ScreenSource, SourceFrame, SourceRefusal};
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::session_state::SessionState;
use crate::screen::core::window_list::SourceId;

/// `WM_STATE` value of a window in the normal state (ICCCM 4.1.3.1).
const WM_STATE_NORMAL: u32 = 1;

/// FNV-1a over 64-bit words of the picture (the tail is zero-padded).
pub(crate) fn picture_hash(data: &[u8]) -> u64 {
    const START: u64 = 0xCBF2_9CE4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01B3;
    let mut hash = START;
    let mut words = data.chunks_exact(8);
    for word in &mut words {
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(word);
        hash = (hash ^ u64::from_le_bytes(bytes)).wrapping_mul(PRIME);
    }
    let rest = words.remainder();
    if !rest.is_empty() {
        let mut bytes = [0u8; 8];
        bytes[..rest.len()].copy_from_slice(rest);
        hash = (hash ^ u64::from_le_bytes(bytes)).wrapping_mul(PRIME);
    }
    hash
}

fn failed(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("x11: {error}"))
}

/// A request that cannot be sent: the connection (`DISPLAY`) is lost.
fn lost(error: x11rb::errors::ConnectionError) -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, error)
}

/// A connection error ends the session (`BrokenPipe`); an error about a
/// window means it is gone.
fn reply_error(error: ReplyError, window: bool) -> io::Error {
    match error {
        ReplyError::ConnectionError(error) => io::Error::new(io::ErrorKind::BrokenPipe, error),
        ReplyError::X11Error(error)
            if window
                && matches!(
                    error.error_kind,
                    ErrorKind::Window | ErrorKind::Drawable | ErrorKind::Pixmap
                ) =>
        {
            io::Error::new(io::ErrorKind::NotFound, "Das Fenster existiert nicht mehr")
        }
        ReplyError::X11Error(error) => failed(format!("{:?}", error.error_kind)),
    }
}

/// A memfd shared with the X server.
struct Segment {
    id: u32,
    map: *mut c_void,
    len: usize,
}

// SAFETY: the mapping is only read through `&self`, by the owner.
unsafe impl Send for Segment {}

impl Segment {
    fn create(conn: &RustConnection, len: usize) -> io::Result<Self> {
        // SAFETY: plain system calls; the descriptor is owned right away.
        let (fd, map) = unsafe {
            let raw = libc::memfd_create(c"se-screen".as_ptr(), libc::MFD_CLOEXEC);
            if raw < 0 {
                return Err(io::Error::last_os_error());
            }
            let fd = OwnedFd::from_raw_fd(raw);
            if libc::ftruncate(raw, len as libc::off_t) != 0 {
                return Err(io::Error::last_os_error());
            }
            let map = libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ,
                libc::MAP_SHARED,
                raw,
                0,
            );
            if map == libc::MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            (fd, map)
        };
        // From here on the mapping goes away with the segment.
        let mut segment = Self { id: 0, map, len };
        segment.id = conn.generate_id().map_err(failed)?;
        // The server gets its own descriptor; ours closes with `fd`.
        let shared = fd.try_clone()?;
        conn.shm_attach_fd(segment.id, shared, false)
            .map_err(failed)?
            .check()
            .map_err(failed)?;
        Ok(segment)
    }

    fn bytes(&self) -> &[u8] {
        // SAFETY: `map` maps `len` readable bytes until `release`.
        unsafe { std::slice::from_raw_parts(self.map.cast::<u8>(), self.len) }
    }

    /// Tells the server to let go of the segment, then unmaps it.
    fn release(self, conn: &RustConnection) {
        if self.id != 0 {
            let _ = conn.shm_detach(self.id);
        }
    }
}

impl Drop for Segment {
    fn drop(&mut self) {
        // SAFETY: the mapping is ours and nothing refers to it any more.
        unsafe { libc::munmap(self.map, self.len) };
    }
}

/// Facts of the X server for the host's limits.
pub(crate) struct ServerFacts {
    pub shm: bool,
    pub compositor: bool,
}

/// Whether the server shares memory by descriptor (MIT-SHM 1.2).
fn shm_usable(conn: &RustConnection) -> bool {
    conn.shm_query_version()
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .is_some_and(|version| (version.major_version, version.minor_version) >= (1, 2))
}

/// Whether a compositing manager owns `_NET_WM_CM_S<n>` and Composite is
/// there.
fn compositor_running(conn: &RustConnection, atoms: &Atoms) -> bool {
    let owner = conn
        .get_selection_owner(atoms.wm_cm_selection)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .is_some_and(|reply| reply.owner != 0);
    owner
        && conn
            .composite_query_version(0, 4)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some()
}

/// Looks at the server of `DISPLAY`; `None` without a server.
pub(crate) fn probe() -> Option<ServerFacts> {
    let (conn, screen) = RustConnection::connect(None).ok()?;
    let atoms = Atoms::intern(&conn, screen).ok()?;
    Some(ServerFacts {
        shm: shm_usable(&conn),
        compositor: compositor_running(&conn, &atoms),
    })
}

enum Target {
    Monitor { id: String, x: i32, y: i32 },
    Window(Window),
}

/// What one pass over the target found.
enum Pass {
    Paused(SessionState),
    Image(BgraImage, i32, i32),
}

/// A monitor or window of the X11 session.
pub(crate) struct X11Source {
    conn: RustConnection,
    root: Window,
    atoms: Atoms,
    legacy_state: Atom,
    target: Target,
    size: FrameSize,
    shm: bool,
    compositor: bool,
    cursor: bool,
    segment: Option<Segment>,
    last_hash: Option<u64>,
}

fn refuse(reason: &str) -> SourceRefusal {
    SourceRefusal::new(reason)
}

impl X11Source {
    /// Opens `source` on the server of `DISPLAY`.
    pub(crate) fn open(source: &SourceId) -> Result<Self, SourceRefusal> {
        let (conn, screen_number) =
            RustConnection::connect(None).map_err(|_| refuse("no_display"))?;
        let screen = &conn.setup().roots[screen_number];
        let root = screen.root;
        let format = conn
            .setup()
            .pixmap_formats
            .iter()
            .find(|format| format.depth == screen.root_depth);
        if format.is_none_or(|format| format.bits_per_pixel != 32)
            || conn.setup().image_byte_order != ImageOrder::LSB_FIRST
        {
            return Err(refuse("x11_unsupported_pixel_format"));
        }
        let atoms = Atoms::intern(&conn, screen_number).map_err(|e| refuse(&e.to_string()))?;
        let legacy_state = conn
            .intern_atom(false, b"WM_STATE")
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map_or(0, |reply| reply.atom);
        let shm = shm_usable(&conn);
        let compositor = compositor_running(&conn, &atoms);
        let cursor = x11_cursor::enable(&conn).is_ok();
        let mut opened = Self {
            conn,
            root,
            atoms,
            legacy_state,
            target: Target::Window(0),
            size: FrameSize::new(0, 0),
            shm,
            compositor,
            cursor,
            segment: None,
            last_hash: None,
        };
        opened.aim(source)?;
        Ok(opened)
    }

    /// Resolves the source to a target and its size.
    fn aim(&mut self, source: &SourceId) -> Result<(), SourceRefusal> {
        match source {
            SourceId::Monitor(id) => {
                let monitors = x11_monitors::list(&self.conn, self.root)
                    .map_err(|error| refuse(&error.to_string()))?;
                let monitor = monitors
                    .iter()
                    .find(|monitor| &monitor.id == id)
                    .ok_or_else(|| refuse("monitor_not_found"))?;
                self.size = FrameSize::new(monitor.width, monitor.height);
                self.target = Target::Monitor {
                    id: id.clone(),
                    x: monitor.x,
                    y: monitor.y,
                };
            }
            SourceId::Window(id) => {
                let window =
                    x11_windows::parse_window_id(id).ok_or_else(|| refuse("window_not_found"))?;
                let (_, _, width, height) =
                    x11_windows::root_geometry(&self.conn, self.root, window)
                        .map_err(|_| refuse("window_not_found"))?;
                self.size = FrameSize::new(u32::from(width), u32::from(height));
                self.target = Target::Window(window);
            }
            SourceId::Synthetic => return Err(refuse("capture_unavailable")),
        }
        if self.size.check_frame().is_err() {
            return Err(refuse("source_exceeds_frame_limit"));
        }
        Ok(())
    }

    fn ensure_segment(&mut self, len: usize) -> io::Result<()> {
        if self
            .segment
            .as_ref()
            .is_some_and(|segment| segment.len == len)
        {
            return Ok(());
        }
        if let Some(old) = self.segment.take() {
            old.release(&self.conn);
        }
        self.segment = Some(Segment::create(&self.conn, len)?);
        Ok(())
    }

    /// The bytes of the rectangle (`x`, `y`, `size`) of `drawable` as
    /// 32-bit pixels with opaque alpha.
    fn read(
        &mut self,
        drawable: u32,
        x: i16,
        y: i16,
        size: FrameSize,
        window: bool,
    ) -> io::Result<Vec<u8>> {
        let (width, height, len) = (size.width as u16, size.height as u16, size.bgra_len());
        let mut data = vec![0u8; len];
        if self.shm {
            self.ensure_segment(len)?;
            let id = self.segment.as_ref().map_or(0, |segment| segment.id);
            let format = u8::from(ImageFormat::Z_PIXMAP);
            self.conn
                .shm_get_image(drawable, x, y, width, height, !0, format, id, 0)
                .map_err(lost)?
                .reply()
                .map_err(|error| reply_error(error, window))?;
            if let Some(segment) = &self.segment {
                data.copy_from_slice(&segment.bytes()[..len]);
            }
        } else {
            let reply = self
                .conn
                .get_image(ImageFormat::Z_PIXMAP, drawable, x, y, width, height, !0)
                .map_err(lost)?
                .reply()
                .map_err(|error| reply_error(error, window))?;
            let bytes = reply.data.get(..len).ok_or_else(|| failed("short image"))?;
            data.copy_from_slice(bytes);
        }
        for pixel in data.chunks_exact_mut(4) {
            pixel[3] = 0xFF;
        }
        Ok(data)
    }

    /// The window state `Normal`; a window without `WM_STATE` counts so.
    fn is_normal(&self, window: Window) -> io::Result<bool> {
        if self.legacy_state == 0 {
            return Ok(true);
        }
        let reply = self
            .conn
            .get_property(false, window, self.legacy_state, self.legacy_state, 0, 1)
            .map_err(lost)?
            .reply()
            .map_err(|error| reply_error(error, true))?;
        Ok(reply
            .value32()
            .and_then(|mut v| v.next())
            .is_none_or(|state| state == WM_STATE_NORMAL))
    }

    fn pass_monitor(&mut self, x: i32, y: i32) -> io::Result<Pass> {
        let size = self.size;
        let data = self.read(self.root, x as i16, y as i16, size, false)?;
        Ok(Pass::Image(BgraImage::from_vec(size, data)?, x, y))
    }

    fn pass_window(&mut self, window: Window) -> io::Result<Pass> {
        let attributes = self
            .conn
            .get_window_attributes(window)
            .map_err(lost)?
            .reply()
            .map_err(|error| reply_error(error, true))?;
        if attributes.map_state != MapState::VIEWABLE
            || x11_windows::is_hidden(&self.conn, &self.atoms, window)?
            || !self.is_normal(window)?
        {
            return Ok(Pass::Paused(SessionState::WindowMinimized));
        }
        let (x, y, width, height) = x11_windows::root_geometry(&self.conn, self.root, window)?;
        let size = FrameSize::new(u32::from(width), u32::from(height));
        size.check_frame()?;
        self.size = size;
        if self.compositor {
            if let Ok(data) = self.read_pixmap(window, size) {
                return Ok(Pass::Image(BgraImage::from_vec(size, data)?, x, y));
            }
        }
        let data = self.read(self.root, x as i16, y as i16, size, true)?;
        Ok(Pass::Image(BgraImage::from_vec(size, data)?, x, y))
    }

    /// The window's own pixels from the compositor's pixmap.
    fn read_pixmap(&mut self, window: Window, size: FrameSize) -> io::Result<Vec<u8>> {
        let pixmap = self.conn.generate_id().map_err(failed)?;
        self.conn
            .composite_name_window_pixmap(window, pixmap)
            .map_err(lost)?
            .check()
            .map_err(|error| reply_error(error, true))?;
        let data = self.read(pixmap, 0, 0, size, true);
        let _ = self.conn.free_pixmap(pixmap);
        data
    }

    /// The monitor's place and size may change (RandR); look it up again.
    /// `false` when it is gone or its picture would exceed the frame limits.
    fn refresh_monitor(&mut self, id: &str) -> bool {
        let Ok(monitors) = x11_monitors::list(&self.conn, self.root) else {
            return false;
        };
        let Some(monitor) = monitors.iter().find(|monitor| monitor.id == id) else {
            return false;
        };
        let size = FrameSize::new(monitor.width, monitor.height);
        if !size.is_valid_frame() {
            return false;
        }
        self.size = size;
        self.target = Target::Monitor {
            id: id.to_string(),
            x: monitor.x,
            y: monitor.y,
        };
        true
    }

    fn pass(&mut self) -> io::Result<Pass> {
        match &self.target {
            Target::Monitor { id, x, y } => {
                let (id, x, y) = (id.clone(), *x, *y);
                match self.pass_monitor(x, y) {
                    Err(error) if error.kind() != io::ErrorKind::BrokenPipe => {
                        // The area may no longer fit the root: once more
                        // with the monitor as it is now.
                        if self.refresh_monitor(&id) {
                            if let Target::Monitor { x, y, .. } = self.target {
                                return self.pass_monitor(x, y);
                            }
                        }
                        Err(error)
                    }
                    other => other,
                }
            }
            Target::Window(window) => {
                let window = *window;
                self.pass_window(window)
            }
        }
    }
}

impl ScreenSource for X11Source {
    fn size(&self) -> FrameSize {
        self.size
    }

    fn capture(&mut self, _frame_id: u64, keyframe: bool) -> io::Result<SourceFrame> {
        let (mut image, x, y) = match self.pass()? {
            Pass::Paused(state) => return Ok(SourceFrame::Paused(state)),
            Pass::Image(image, x, y) => (image, x, y),
        };
        if self.cursor {
            if let Ok(cursor) = x11_cursor::fetch(&self.conn) {
                let size = image.size();
                x11_cursor::blend(image.data_mut(), size, x, y, &cursor);
            }
        }
        let hash = picture_hash(image.data());
        let same = self.last_hash == Some(hash);
        self.last_hash = Some(hash);
        if same && !keyframe {
            return Ok(SourceFrame::Unchanged);
        }
        Ok(SourceFrame::Frame(image))
    }
}

impl Drop for X11Source {
    fn drop(&mut self) {
        if let Some(segment) = self.segment.take() {
            segment.release(&self.conn);
        }
    }
}

#[cfg(test)]
#[path = "x11_tests.rs"]
mod tests;
