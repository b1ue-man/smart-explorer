//! Screen capture on X11 (SCR-M5, U19-6): a monitor or a window of the
//! session of `DISPLAY`, read with MIT-SHM into a memfd that the server
//! shares (`attach_fd`), or with plain `GetImage` when the server has no
//! usable MIT-SHM (the limit `x11_no_shm` says so). Windows come from their
//! composite pixmap when a compositor runs. Explicit raw windows may fall back
//! to the root crop (`x11_window_overlaps_without_compositor`); family members
//! require their own pixmap and fail without it. The pointer is blended in
//! from XFixes. DAMAGE is not
//! used (compositors hide the root's damage); a picture equal to the last one
//! by hash and exact pixel comparison is `Unchanged`.
use super::x11_cursor;
use super::x11_monitors;
use super::x11_windows::{self, Atoms};
use crate::screen::core::capture_admission::{
    CaptureGeneration, CaptureGrant, CaptureOwner, SessionAdmission, SourceKey,
};
use crate::screen::core::capture_image::{AdmittedBytes, CaptureImageBuilder, CapturedImage};
use crate::screen::core::host_backend::{HostOutputPlan, PreparedSourceInfo};
use crate::screen::core::host_backend::{ScreenSource, SourceFrame, SourceRefusal};
#[cfg(test)]
use crate::screen::core::pixel_convert::BgraImage;
use crate::screen::core::pixel_convert::FrameSize;
use std::ffi::c_void;
use std::io;
use std::os::fd::{FromRawFd, OwnedFd};
use std::sync::Arc;
use x11rb::connection::Connection;
use x11rb::errors::ReplyError;
use x11rb::protocol::composite::ConnectionExt as _;
use x11rb::protocol::shm::ConnectionExt as _;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, ImageFormat, ImageOrder, MapState, Window,
};
use x11rb::protocol::ErrorKind;
use x11rb::rust_connection::RustConnection;
#[path = "x11_admission_stream.rs"]
pub(super) mod admission_stream;
use admission_stream::{AdmissionStream, CaptureConnection, Gate};
#[path = "x11_capture_shm.rs"]
mod capture_shm;
use capture_shm::Segment;
#[path = "x11_capture_memory.rs"]
mod capture_memory;
#[path = "x11_capture_read.rs"]
mod capture_read;
#[path = "x11_capture_source.rs"]
mod capture_source;
use crate::screen::core::frame_budget::FrameLimits;
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
/// Facts of the X server for the host's limits.
pub(crate) struct ServerFacts {
    pub shm: bool,
    pub compositor: bool,
}

/// Whether the server shares memory by descriptor (MIT-SHM 1.2).
fn shm_usable<C: Connection>(conn: &C) -> bool {
    conn.shm_query_version()
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .is_some_and(|version| (version.major_version, version.minor_version) >= (1, 2))
}

/// Whether a compositing manager owns `_NET_WM_CM_S<n>` and Composite is
/// there.
fn compositor_running<C: Connection>(conn: &C, atoms: &Atoms) -> bool {
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

#[path = "x11_capture_target.rs"]
mod capture_target;
use capture_target::Target;

/// What one pass over the target found.
enum Pass {
    Paused(SessionState),
    Image(CaptureImageBuilder, i32, i32),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WindowPixels {
    RawFallback,
    OwnPixmap,
}

/// A monitor or window of the X11 session.
pub(crate) struct X11Source {
    conn: CaptureConnection,
    gate: Gate,
    admission: Arc<SessionAdmission>,
    source_key: SourceKey,
    generation: CaptureGeneration,
    grant: Option<CaptureGrant>,
    output: Option<HostOutputPlan>,
    root: Window,
    atoms: Atoms,
    legacy_state: Atom,
    target: Target,
    source: SourceId,
    geometry: Option<super::x11_input::Area>,
    geometry_changed: bool,
    reference_generation: u64,
    size: FrameSize,
    shm: bool,
    compositor: bool,
    pixmap_bytes: u64,
    window_pixels: WindowPixels,
    cursor: bool,
    segment: Option<Segment>,
    last_hash: Option<u64>,
    last_image: Option<CapturedImage>,
}
fn refuse(reason: &str) -> SourceRefusal {
    SourceRefusal::new(reason)
}

impl X11Source {
    #[cfg(test)]
    pub(crate) fn open(source: &SourceId) -> Result<Self, SourceRefusal> {
        let admission = super::host_backend::prepared::test_admission("x11-fixture");
        let key = SourceKey::new("x11-fixture", &format!("linux:{source:?}"))
            .map_err(|error| refuse(&error.to_string()))?;
        let prepared = Self::prepare(source, key, admission.clone(), false)?;
        let plan = prepared
            .plan(None)
            .map_err(|error| refuse(&error.to_string()))?;
        let grant = admission
            .authority()
            .approve(plan)
            .map_err(|error| refuse(&error.to_string()))?;
        prepared.activate(grant, None)
    }
    pub(super) fn prepare(
        source: &SourceId,
        source_key: SourceKey,
        admission: Arc<SessionAdmission>,
        family: bool,
    ) -> Result<Self, SourceRefusal> {
        let (conn, screen_number, gate) =
            AdmissionStream::connect().map_err(|_| refuse("no_display"))?;
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
            gate,
            admission,
            source_key,
            generation: CaptureGeneration::new(1),
            grant: None,
            output: None,
            root,
            atoms,
            legacy_state,
            target: Target::Window(0),
            source: source.clone(),
            geometry: None,
            geometry_changed: false,
            reference_generation: 0,
            size: FrameSize::new(0, 0),
            shm,
            compositor,
            pixmap_bytes: 0,
            window_pixels: WindowPixels::RawFallback,
            cursor,
            segment: None,
            last_hash: None,
            last_image: None,
        };
        opened.aim(source)?;
        if family {
            if !opened.compositor {
                return Err(refuse("x11_family_capture_requires_compositor"));
            }
            opened.window_pixels = WindowPixels::OwnPixmap;
        }
        Ok(opened)
    }

    pub(super) fn prepared_info(&self) -> PreparedSourceInfo {
        PreparedSourceInfo {
            native_size: Some(self.size),
            capture_limits: FrameLimits::RESOURCE,
            supports_regions: true,
            state: SessionState::Starting,
            generation: self.generation,
        }
    }
    pub(super) fn activate(
        mut self,
        grant: CaptureGrant,
        output: Option<HostOutputPlan>,
    ) -> Result<Self, SourceRefusal> {
        let checked = (|| {
            if self.current_area()?
                != self
                    .geometry
                    .ok_or_else(|| failed("missing prepared geometry"))?
            {
                return Err(failed("x11_prepared_geometry_changed"));
            }
            if self.window_pixels == WindowPixels::OwnPixmap
                && !compositor_running(&self.conn, &self.atoms)
            {
                return Err(failed("x11_family_capture_requires_compositor"));
            }
            capture_memory::verify(&grant, &self.plan(output.as_ref())?)
        })();
        checked.map_err(|error| refuse(&error.to_string()))?;
        self.pixmap_bytes = match self.target {
            Target::Window(window) if self.compositor => {
                self.pixmap_geometry(window)
                    .map_err(|e| refuse(&e.to_string()))?
                    .2
            }
            _ => 0,
        };
        self.grant = Some(grant);
        self.output = output;
        Ok(self)
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
        Ok(Pass::Image(data, x, y))
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
        FrameLimits::RESOURCE.check(size)?;
        let area = super::x11_input::Area {
            x,
            y,
            width: i32::from(width),
            height: i32::from(height),
        };
        self.admit_geometry(area)?;
        if self.window_pixels == WindowPixels::OwnPixmap {
            // Compositors may disappear after selection. NameWindowPixmap must
            // still name this member's current storage; never substitute root.
            if !compositor_running(&self.conn, &self.atoms) {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "x11_family_capture_requires_compositor",
                ));
            }
            let data = self.read_pixmap(window, size).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("x11_family_member_pixmap_failed: {error}"),
                )
            })?;
            return Ok(Pass::Image(data, x, y));
        }
        if self.compositor {
            if let Ok(data) = self.read_pixmap(window, size) {
                return Ok(Pass::Image(data, x, y));
            }
        }
        let data = self.read(self.root, x as i16, y as i16, size, true)?;
        Ok(Pass::Image(data, x, y))
    }
}

#[cfg(test)]
#[path = "x11_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "batches_capture_recovery_task_tests.rs"]
mod recovery_tests;
