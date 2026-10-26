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
use std::ffi::c_void;
use std::io;
use std::sync::Arc;
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
use crate::screen::core::pixel_convert::FrameSize;
#[cfg(test)]
use crate::screen::core::pixel_convert::BgraImage;
use crate::screen::core::capture_admission::{CaptureGeneration, CaptureGrant, CaptureOwner, SessionAdmission, SourceKey};
use crate::screen::core::capture_image::{AdmittedBytes, CaptureImageBuilder, CapturedImage};
use crate::screen::core::host_backend::{PreparedSourceInfo, HostOutputPlan};
#[path="x11_admission_stream.rs"]
pub(super) mod admission_stream;
use admission_stream::{AdmissionStream, CaptureConnection, Gate};
#[path="x11_capture_shm.rs"]
mod capture_shm;
use capture_shm::Segment;
#[path="x11_capture_memory.rs"]
mod capture_memory;
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

#[derive(Clone,Copy,PartialEq,Eq)]
enum WindowPixels { RawFallback, OwnPixmap }

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
    source:SourceId,
    geometry:Option<super::x11_input::Area>,
    geometry_changed:bool,
    reference_generation:u64,
    size: FrameSize,
    shm: bool,
    compositor: bool,
    pixmap_bytes:u64,
    window_pixels:WindowPixels,
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
    pub(crate) fn open(source:&SourceId)->Result<Self,SourceRefusal> {
        let admission=super::host_backend::prepared::test_admission("x11-fixture");
        let key=SourceKey::new("x11-fixture",&format!("linux:{source:?}"))
            .map_err(|error|refuse(&error.to_string()))?;
        let prepared=Self::prepare(source,key,admission.clone(),false)?;
        let plan=prepared.plan(None).map_err(|error|refuse(&error.to_string()))?;
        let grant=admission.authority().approve(plan).map_err(|error|refuse(&error.to_string()))?;
        prepared.activate(grant,None)
    }
    pub(super) fn prepare(source: &SourceId, source_key: SourceKey,
        admission: Arc<SessionAdmission>, family: bool) -> Result<Self, SourceRefusal> {
        let (conn, screen_number, gate) = AdmissionStream::connect().map_err(|_|refuse("no_display"))?;
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
            conn, gate, admission, source_key, generation: CaptureGeneration::new(1),
            grant: None, output: None,
            root,
            atoms,
            legacy_state,
            target: Target::Window(0),
            source:source.clone(),geometry:None,geometry_changed:false,reference_generation:0,
            size: FrameSize::new(0, 0),
            shm,
            compositor,pixmap_bytes:0,
            window_pixels:WindowPixels::RawFallback,
            cursor,
            segment: None,
            last_hash: None,
            last_image: None,
        };
        opened.aim(source)?;
        if family {
            if !opened.compositor {return Err(refuse("x11_family_capture_requires_compositor"));}
            opened.window_pixels=WindowPixels::OwnPixmap;
        }
        Ok(opened)
    }


    pub(super) fn prepared_info(&self)->PreparedSourceInfo {
        PreparedSourceInfo {native_size:Some(self.size),capture_limits:FrameLimits::RESOURCE,
            supports_regions:true,state:SessionState::Starting,generation:self.generation}
    }
    pub(super) fn activate(mut self,grant:CaptureGrant,output:Option<HostOutputPlan>)->Result<Self,SourceRefusal> {
        let checked=(|| {
            if self.current_area()?!=self.geometry.ok_or_else(||failed("missing prepared geometry"))? {
                return Err(failed("x11_prepared_geometry_changed"));
            }
            if self.window_pixels==WindowPixels::OwnPixmap && !compositor_running(&self.conn,&self.atoms) {
                return Err(failed("x11_family_capture_requires_compositor"));
            }
            capture_memory::verify(&grant,&self.plan(output.as_ref())?)
        })();
        checked.map_err(|error|refuse(&error.to_string()))?;
        self.pixmap_bytes=match self.target {Target::Window(window) if self.compositor=>self.pixmap_geometry(window).map_err(|e|refuse(&e.to_string()))?.2,_=>0};
        self.grant=Some(grant);self.output=output;
        Ok(self)
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
        self.segment = Some(Segment::create(&self.conn, len, self.grant.as_ref().ok_or_else(||failed("capture_not_activated"))?)?);
        Ok(())
    }

    /// The bytes of the rectangle (`x`, `y`, `size`) of `drawable` as
    /// 32-bit pixels with opaque alpha.
    fn read(&mut self, drawable: u32, x: i16, y: i16, size: FrameSize,
        window: bool) -> io::Result<CaptureImageBuilder> {
        let len=FrameLimits::RESOURCE.check(size)?;
        let grant=self.grant.clone().ok_or_else(||failed("capture_not_activated"))?;
        let (width,height)=(size.width as u16,size.height as u16);
        let bytes = if self.shm {
            self.ensure_segment(len)?;
            let mut bytes=AdmittedBytes::new(&grant,CaptureOwner::SourcePixels,len as u64)?;
            let segment=self.segment.as_ref().ok_or_else(||failed("missing SHM segment"))?;
            self.conn.shm_get_image(drawable,x,y,width,height,!0,u8::from(ImageFormat::Z_PIXMAP),segment.id,0)
                .map_err(lost)?.reply().map_err(|error|reply_error(error,window))?;
            bytes.as_mut_slice().copy_from_slice(&segment.bytes()[..len]);
            bytes
        } else {
            // The closure claims parsed reply storage before x11rb's to_vec.
            // The stream separately holds its raw PacketReader promise.
            let bytes=AdmittedBytes::from_allocator(&grant,CaptureOwner::SourcePixels,len as u64,|| {
                let fence=self.conn.get_input_focus().map_err(lost)?;
                let sequence=fence.sequence_number().checked_add(1).ok_or_else(||failed("x11 sequence exhausted"))?;
                fence.reply().map_err(|error|reply_error(error,window))?;
                self.gate.image(sequence,len as u64,&grant)?;
                let cookie=self.conn.get_image(ImageFormat::Z_PIXMAP,drawable,x,y,width,height,!0).map_err(lost)?;
                if cookie.sequence_number()!=sequence {return Err(failed("x11 image sequence changed"));}
                let reply=cookie.reply().map_err(|error|reply_error(error,window))?;
                self.gate.finish()?;
                if reply.data.len()!=len {return Err(failed("short image"));}
                Ok(reply.data)
            })?;
            bytes
        };
        let mut image=CaptureImageBuilder::from_bytes(size,FrameLimits::RESOURCE,&grant,bytes)?;
        for pixel in image.pixels_mut().chunks_exact_mut(4) {pixel[3]=255;}
        Ok(image)
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
        let area=super::x11_input::Area {x,y,width:i32::from(width),height:i32::from(height)};
        self.admit_geometry(area)?;
        if self.window_pixels==WindowPixels::OwnPixmap {
            // Compositors may disappear after selection. NameWindowPixmap must
            // still name this member's current storage; never substitute root.
            if !compositor_running(&self.conn,&self.atoms) {
                return Err(io::Error::new(io::ErrorKind::Unsupported,"x11_family_capture_requires_compositor"));
            }
            let data=self.read_pixmap(window,size).map_err(|error|
                io::Error::new(error.kind(),format!("x11_family_member_pixmap_failed: {error}")))?;
            return Ok(Pass::Image(data,x,y));
        }
        if self.compositor {
            if let Ok(data) = self.read_pixmap(window, size) {
                return Ok(Pass::Image(data, x, y));
            }
        }
        let data = self.read(self.root, x as i16, y as i16, size, true)?;
        Ok(Pass::Image(data, x, y))
    }

    /// The window's own pixels from the compositor's pixmap.
    fn read_pixmap(&mut self, window: Window, size: FrameSize) -> io::Result<CaptureImageBuilder> {
        // No Parent call while the server is grabbed. A changed border or
        // geometry is rejected here and readmitted on the next pass.
        let expected=self.pixmap_geometry(window)?;
        if expected.1!=size || expected.2!=self.pixmap_bytes {return Err(failed("x11_pixmap_geometry_changed"));}
        let bytes=expected.2;
        let grant=self.grant.clone().ok_or_else(||failed("capture_not_activated"))?;
        let slice=grant.claim(CaptureOwner::NativeSurface,bytes)?;
        self.conn.grab_server().map_err(lost)?.check().map_err(|e|reply_error(e,true))?;
        let named=(|| {
            if self.pixmap_geometry(window)?!=expected {return Err(failed("x11_pixmap_geometry_changed"));}
            let pixmap=self.conn.generate_id().map_err(failed)?;
            self.conn.composite_name_window_pixmap(window,pixmap).map_err(lost)?.check()
                .map_err(|e|reply_error(e,true))?;
            Ok(pixmap)
        })();
        let ungrab=self.conn.ungrab_server().map_err(lost).and_then(|c|c.check().map_err(|e|reply_error(e,true)));
        let pixmap=match named {
            Ok(pixmap)=>pixmap,
            Err(error)=>{if error.kind()==io::ErrorKind::BrokenPipe {self.gate.retain(slice);}ungrab?;return Err(error);}
        };
        // The named storage includes borders; read only the original interior.
        let data=match ungrab {
            Ok(())=>self.read(pixmap,expected.0 as i16,expected.0 as i16,size,true),
            Err(error)=>Err(error),
        };
        let freed=self.conn.free_pixmap(pixmap).map_err(lost)
            .and_then(|c|c.check().map_err(|e|reply_error(e,true)));
        if freed.is_err() {self.gate.retain(slice);} else {drop(slice);}
        freed?;
        data
    }




}
impl ScreenSource for X11Source {
    fn capture_grant(&self)->Option<CaptureGrant> {self.grant.clone()}
    fn legacy_input_invalid(&self)->bool {self.geometry_changed}
    fn reference_generation(&self)->u64 {self.reference_generation}
    fn inject_input(&mut self,_backend:&dyn crate::screen::core::host_backend::ScreenHostBackend,
        reference:u64,presented:bool,events:&[crate::screen::core::input_model::InputEvent])
        ->Result<(),crate::screen::core::wire_control::ControlBlock> {
        use crate::screen::core::wire_control::ControlBlock;
        if reference!=self.reference_generation{return Err(ControlBlock::ControlUnavailable);}
        let current=self.current_area().map_err(|_|ControlBlock::ControlUnavailable)?;
        if self.geometry!=Some(current) {
            self.geometry_changed=true;
            return Err(ControlBlock::ControlUnavailable);
        }
        if self.last_image.is_none()||(!presented&&self.geometry_changed){return Err(ControlBlock::ControlUnavailable);}
        super::x11_input::inject_bound(&self.source,current,events)
    }
    fn frame_limits(&self)->FrameLimits {FrameLimits::RESOURCE}
    fn size(&self) -> FrameSize {
        self.size
    }

    fn capture(&mut self, _frame_id: u64, keyframe: bool) -> io::Result<SourceFrame> {
        let (mut image, x, y) = match self.pass()? {
            Pass::Paused(state) => return Ok(SourceFrame::Paused(state)),
            Pass::Image(image, x, y) => (image, x, y),
        };
        if self.cursor {
            if let Ok(cursor) = x11_cursor::fetch(&self.conn,&self.gate,self.grant.as_ref().ok_or_else(||failed("capture_not_activated"))?,self.admission.authority()) {
                let size = self.size;
                x11_cursor::blend(image.pixels_mut(), size, x, y, &cursor.image);
            }
        }
        let image=image.freeze();
        let captured=super::x11_input::Area {x,y,width:image.size().width as i32,height:image.size().height as i32};
        let current=self.current_area()?;
        self.admit_geometry(current)?;
        if current!=captured{return Ok(SourceFrame::Paused(SessionState::Starting));}
        let hash = picture_hash(image.image().data());
        let same = self.last_hash == Some(hash) && self.last_image.as_ref().is_some_and(|last|
            last.size()==image.size() && last.image().data()==image.image().data());
        self.last_hash = Some(hash);
        if !same { self.last_image=Some(image.clone()); }
        if same {
            return if keyframe {Ok(SourceFrame::Frame(self.last_image.clone().ok_or_else(||failed("missing unchanged cache"))?))}
                else {Ok(SourceFrame::Unchanged)};
        }
        Ok(SourceFrame::Frame(image))
    }
}
impl Drop for X11Source {
    fn drop(&mut self) {
        if let Some(segment) = self.segment.take() {
            segment.release(&self.conn);
        }
        self.last_image=None;
        if let Some(grant)=&self.grant {
            if !grant.is_member() {
                if let Err(error)=grant.retire() {eprintln!("X11 capture retirement: {error}");}
            }
        }
    }
}

#[cfg(test)]
#[path = "x11_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "batches_capture_recovery_task_tests.rs"]
mod recovery_tests;
