//! Windows Graphics Capture: metadata and an exact grant precede pool creation.
//! Cached and returned pictures are aliases of immutable admitted storage.
//! Plan 2026-11-14 bildkette: the capture waits for the pool's `FrameArrived`
//! instead of polling (B-10) and limits the session to the pipeline's frame
//! interval with `MinUpdateInterval` where Windows has it (B-11); for whole
//! monitors it is the fallback of Desktop Duplication (B-12).
use super::capture_prepared::recovery::CaptureRecovery;
use super::capture_retirement::CaptureResources;
use super::capture_slots::{CaptureBudget, CaptureGeometry, CaptureKind};
use super::capture_source::{CapturePath, Grabber, FIRST_FRAME_WAIT};
use super::d3d_device::{self, DeviceSet, Read, Readback};
use super::monitors;
use crate::screen::core::capture_admission::CaptureGrant;
use crate::screen::core::capture_picture::{Picture, SourcePicture};
use crate::screen::core::host_backend::SourceFrame;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::session_state::SessionState;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Foundation::{EventRegistrationToken, TypedEventHandler};
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
/// Time a session whose pointer capture was switched (pointer overlay, plan
/// 2026-11-14 P6) gets to deliver a picture in the new state before it is
/// started again: on a screen that stands still it delivers none.
const POINTER_SWITCH_WAIT: Duration = Duration::from_millis(150);
#[cfg(any(test, feature = "test-support", debug_assertions))]
static FORCED_OFF: AtomicBool = AtomicBool::new(false);
#[cfg(any(test, feature = "test-support", debug_assertions))]
pub(super) fn force_unavailable_for_tests(off: bool) {
    FORCED_OFF.store(off, Ordering::SeqCst);
}
pub(super) fn available() -> bool {
    #[cfg(any(test, feature = "test-support", debug_assertions))]
    if FORCED_OFF.load(Ordering::SeqCst) {
        return false;
    }
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        GraphicsCaptureSession::IsSupported().unwrap_or(false)
            && windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().is_ok()
    })
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Monitor(String),
    Window(usize),
}

/// CreateForWindow/Monitor supplies only a retained item and metadata. No
/// framepool or capture session exists until activate consumes its grant.
pub(super) struct PreparedWgc {
    devices: DeviceSet,
    item: GraphicsCaptureItem,
    closed: Arc<AtomicBool>,
    closed_token: EventRegistrationToken,
}
impl PreparedWgc {
    pub(super) fn prepare(target: &Target) -> io::Result<Self> {
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(io::Error::other)?;
        let item: GraphicsCaptureItem = match target {
            Target::Monitor(id) => {
                let entry = monitors::find(id).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "capture monitor disappeared")
                })?;
                // SAFETY: the monitor was just enumerated; the item retains this target.
                unsafe { interop.CreateForMonitor(entry.handle) }.map_err(io::Error::other)?
            }
            // SAFETY: invalid window handles cause an error; no desktop fallback.
            Target::Window(handle) => unsafe { interop.CreateForWindow(HWND(*handle as *mut _)) }
                .map_err(io::Error::other)?,
        };
        let closed = Arc::new(AtomicBool::new(false));
        let flag = closed.clone();
        let token = item
            .Closed(&TypedEventHandler::<
                GraphicsCaptureItem,
                windows::core::IInspectable,
            >::new(move |_, _| {
                flag.store(true, Ordering::Release);
                Ok(())
            }))
            .map_err(io::Error::other)?;
        let devices = match d3d_device::create() {
            Ok(devices) => devices,
            Err(error) => {
                let _ = item.RemoveClosed(token);
                return Err(io::Error::other(error));
            }
        };
        Ok(Self {
            devices,
            item,
            closed,
            closed_token: token,
        })
    }
    pub(super) fn geometry(&self) -> io::Result<Option<CaptureGeometry>> {
        if self.closed.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "original WGC source closed",
            ));
        }
        let size = self.item.Size().map_err(io::Error::other)?;
        if size.Width <= 0 || size.Height <= 0 {
            return Ok(None);
        }
        self.geometry_for(FrameSize::new(size.Width as u32, size.Height as u32))
            .map(Some)
    }
    /// The geometry of `native` pixels, also of a frame's own `ContentSize`.
    fn geometry_for(&self, native: FrameSize) -> io::Result<CaptureGeometry> {
        if self.closed.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "original WGC source closed",
            ));
        }
        let limits = self.devices.capture_limits(native)?;
        Ok(CaptureGeometry {
            native,
            delivered: self.devices.delivered_size(native.width, native.height)?,
            limits,
            kind: CaptureKind::Wgc,
        })
    }
}
impl Drop for PreparedWgc {
    fn drop(&mut self) {
        let _ = self.item.RemoveClosed(self.closed_token);
    }
}
struct Live {
    resources: CaptureResources,
    size: FrameSize,
}
impl Live {
    fn close(self) -> io::Result<()> {
        self.resources.close()
    }
}
fn create(target: &PreparedWgc, budget: &CaptureBudget) -> io::Result<Live> {
    let expected = budget.geometry().native;
    let slice = budget.claim_native()?;
    let size = SizeInt32 {
        Width: expected.width as i32,
        Height: expected.height as i32,
    };
    let resources = CaptureResources::create(&target.devices.winrt, &target.item, size, slice)?;
    Ok(Live {
        resources,
        size: expected,
    })
}
pub(super) struct WgcCapture {
    live: Option<Live>,
    target: PreparedWgc,
    readback: Readback,
    /// The newest picture, CPU pixels or on the GPU (BK3b).
    last: Option<Picture>,
    size: FrameSize,
    budget: CaptureBudget,
    recovery: CaptureRecovery,
    /// When the overlay switched this session's pointer capture, until a
    /// picture taken after the switch was read.
    pointer_switched: Option<Instant>,
    /// Gemini round 8, 1: whether `last` was taken with the pointer drawn.
    last_drawn: bool,
    /// The pipeline's frame interval, set again on every new session.
    interval: Option<Duration>,
}
impl WgcCapture {
    pub(super) fn open(target: PreparedWgc, budget: CaptureBudget) -> io::Result<Self> {
        let actual = target
            .geometry()?
            .ok_or_else(|| io::Error::other("WGC native metadata pending"))?;
        if actual.native != budget.geometry().native {
            return Err(io::Error::other("WGC geometry changed before activation"));
        }
        let live = create(&target, &budget)?;
        let size = budget.geometry().displayed();
        Ok(Self {
            live: Some(live),
            target,
            readback: Readback::default(),
            last: None,
            size,
            budget,
            recovery: CaptureRecovery::active(),
            pointer_switched: None,
            last_drawn: false,
            interval: None,
        })
    }
    /// Applies the frame interval to the live session; false where it lacks
    /// `MinUpdateInterval`.
    fn apply_interval(&self) -> bool {
        match (self.interval, &self.live) {
            (Some(interval), Some(live)) => live.resources.set_min_update_interval(interval),
            _ => true,
        }
    }
    fn newest(&self) -> io::Result<Option<Direct3D11CaptureFrame>> {
        let mut newest: Option<Direct3D11CaptureFrame> = None;
        loop {
            let pool = self
                .live
                .as_ref()
                .ok_or_else(|| io::Error::other("WGC capture paused"))?
                .resources
                .pool()?;
            match pool.TryGetNextFrame() {
                Ok(frame) => {
                    if let Some(older) = newest.replace(frame) {
                        let _ = older.Close();
                    }
                }
                Err(error) if error.code().is_ok() => return Ok(newest),
                Err(error) => return newest.map(Some).ok_or_else(|| io::Error::other(error)),
            }
        }
    }
    /// `content` is the `ContentSize` of a frame that no longer fits the
    /// pool: Microsoft recreates the pool for the item's new size and copies
    /// out "a sub-rect using the ContentSize property"; the pool follows the
    /// size the frames report, never a possibly older `item.Size()`.
    fn restart(&mut self, content: Option<FrameSize>) -> io::Result<()> {
        // WindowsSource can resume through reopen after a hidden secure-
        // desktop pause without a preceding backend activity notification.
        self.recovery.request(true);
        self.stop()?;
        let geometry = match content {
            Some(native) => self.target.geometry_for(native)?,
            None => self
                .target
                .geometry()?
                .ok_or_else(|| io::Error::other("WGC native metadata pending"))?,
        };
        self.budget.replace(geometry)?;
        self.live = Some(create(&self.target, &self.budget)?);
        self.size = geometry.displayed();
        // A missing property was reported when the interval was first set.
        let _ = self.apply_interval();
        Ok(())
    }
    fn stop(&mut self) -> io::Result<()> {
        if let Some(live) = self.live.take() {
            if let Err(error) = live.close() {
                self.target.closed.store(true, Ordering::Release);
                return Err(error);
            }
        }
        self.readback.reset();
        self.last = None;
        self.budget.suspend();
        Ok(())
    }
    /// The picture of `frame`, or `Err(size)` with its `ContentSize` when
    /// that differs from the pool.
    fn read(&mut self, frame: &Direct3D11CaptureFrame) -> io::Result<Result<Picture, FrameSize>> {
        let content = frame.ContentSize().map_err(io::Error::other)?;
        if content.Width <= 0 || content.Height <= 0 {
            return Err(io::Error::other("capture frame has no content"));
        }
        let size = FrameSize::new(content.Width as u32, content.Height as u32);
        if self.live.as_ref().map(|live| live.size) != Some(size) {
            return Ok(Err(size));
        }
        let surface = frame.Surface().map_err(io::Error::other)?;
        let access: IDirect3DDxgiInterfaceAccess = surface.cast().map_err(io::Error::other)?;
        // SAFETY: a WGC surface is a D3D11 texture of this frame's device.
        let texture: ID3D11Texture2D =
            unsafe { access.GetInterface() }.map_err(io::Error::other)?;
        // `texture` is a BGRA texture on `self.target.devices.device`, the
        // device the texture encoder shares (`MFT_MESSAGE_SET_D3D_MANAGER`);
        // it stays valid until `frame.Close()` in `capture_waiting`. While a
        // texture encoder is armed the picture stays on the GPU (BK2a, BK3b).
        let devices = &self.target.devices;
        let read = self
            .readback
            .read(devices, &texture, size.width, size.height, &self.budget)?;
        Ok(Ok(match read {
            Read::Cpu(image) => Picture::Cpu(image.freeze()),
            Read::Gpu(picture) => Picture::Gpu(picture),
        }))
    }
}
impl Grabber for WgcCapture {
    fn frame_limits(&self) -> crate::screen::core::frame_budget::FrameLimits {
        self.budget.geometry().limits
    }
    fn capture_grant(&self) -> Option<CaptureGrant> {
        self.budget.grant().ok().cloned()
    }
    fn reference_generation(&self) -> u64 {
        self.budget.generation()
    }
    fn source_available(&self) -> bool {
        !self.target.closed.load(Ordering::Acquire)
    }
    fn size(&self) -> FrameSize {
        self.size
    }
    fn path(&self) -> Option<CapturePath> {
        Some(CapturePath::GraphicsCapture)
    }
    fn set_frame_interval(&mut self, interval: Duration) -> bool {
        self.interval = Some(interval);
        self.apply_interval()
    }
    fn capture(&mut self, keyframe: bool) -> io::Result<SourceFrame> {
        self.capture_picture(keyframe)?.into_source_frame()
    }
    fn capture_picture(&mut self, keyframe: bool) -> io::Result<SourcePicture> {
        let wait = if self.last.is_none() {
            FIRST_FRAME_WAIT
        } else {
            Duration::ZERO
        };
        self.capture_waiting_picture(keyframe, wait, &mut || true)
    }
    fn capture_waiting(
        &mut self,
        keyframe: bool,
        wait: Duration,
        release: &mut dyn FnMut() -> bool,
    ) -> io::Result<SourceFrame> {
        self.capture_waiting_picture(keyframe, wait, release)?
            .into_source_frame()
    }
    fn capture_waiting_picture(
        &mut self,
        keyframe: bool,
        wait: Duration,
        release: &mut dyn FnMut() -> bool,
    ) -> io::Result<SourcePicture> {
        if !self.source_available() {
            return Err(io::Error::new(io::ErrorKind::NotFound, "WGC source closed"));
        }
        let recovery = self.recovery;
        if !recovery.ensure_live(self.live.is_some(), || self.restart(None))? {
            return Ok(SourcePicture::Unchanged);
        }
        // The pictures delivered so far show the pointer as it was before the
        // overlay switched it; one read in this call may still be from before.
        let switched_now = super::cursor_overlay::take_switched();
        if switched_now {
            self.pointer_switched = Some(Instant::now());
        }
        let deadline = Instant::now() + wait;
        loop {
            // Counted before draining: a picture arriving in between wakes
            // the wait below at once.
            let arrivals = self
                .live
                .as_ref()
                .map(|live| live.resources.arrivals().clone());
            let seen = arrivals.as_ref().map_or(0, |arrivals| arrivals.seen());
            if let Some(frame) = self.newest()? {
                if !switched_now {
                    self.pointer_switched = None;
                }
                // A retained external result has its own immutable owner;
                // the source cache must not force a third distinct CPU slot.
                // WGC has no regions, so whether the previous picture was
                // handed out changes nothing here.
                let _ = release();
                self.last = None;
                // Gemini round 8, 1: a read refused for want of a free picture
                // (the encoder still holds one) waits for it a little, as the
                // duplication does (`capture_dxgi::READ_WAIT`), instead of
                // losing the frame on a screen that may then stand still.
                let until = Instant::now() + Duration::from_millis(50);
                let mut image = self.read(&frame);
                while matches!(&image, Err(e) if e.kind() == io::ErrorKind::WouldBlock)
                    && Instant::now() < until
                {
                    std::thread::sleep(Duration::from_millis(1));
                    image = self.read(&frame);
                }
                let closed = frame.Close();
                drop(frame);
                closed.map_err(io::Error::other)?;
                // Pool replacement occurs only after returning the acquired
                // frame, closing the old pool and obtaining the next grant.
                let image = match image {
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        return Ok(SourcePicture::Unchanged)
                    }
                    result => result?,
                };
                let image = match image {
                    Ok(image) => image,
                    Err(content) => {
                        // One rebuild per call: a frame whose size never
                        // matches cannot spin this loop. The new generation
                        // makes the caller discard this call anyway.
                        self.restart(Some(content))?;
                        return Ok(SourcePicture::Unchanged);
                    }
                };
                self.size = image.size();
                self.last = Some(image.clone());
                self.last_drawn = !crate::screen::core::cursor_overlay::worker().separated();
                return Ok(SourcePicture::Frame(image));
            }
            // Gemini round 8, 1: a switch back to the mode the picture was
            // taken in (the viewer's start toggles it twice) needs nothing.
            let drawn = !crate::screen::core::cursor_overlay::worker().separated();
            if drawn == self.last_drawn {
                self.pointer_switched = None;
            }
            // No picture since the switch: the screen stands still and the
            // viewer keeps the pointer of before in its picture (or misses
            // it). A new session starts with a complete picture.
            if self.last.is_some()
                && self
                    .pointer_switched
                    .is_some_and(|at| at.elapsed() >= POINTER_SWITCH_WAIT)
            {
                self.pointer_switched = None;
                self.restart(None)?;
                return Ok(SourcePicture::Unchanged);
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(match &self.last {
                    Some(last) if keyframe => SourcePicture::Frame(last.clone()),
                    Some(_) => SourcePicture::Unchanged,
                    None => SourcePicture::Paused(SessionState::Starting),
                });
            }
            // Until the compositor delivers (`FrameArrived`), the deadline,
            // or the moment a pointer switch needs a new session.
            let mut until = deadline;
            if let (Some(at), Some(_)) = (self.pointer_switched, &self.last) {
                until = until.min(at + POINTER_SWITCH_WAIT);
            }
            match arrivals {
                Some(arrivals) => arrivals.wait_after(seen, until.saturating_duration_since(now)),
                None => return Ok(SourcePicture::Unchanged),
            }
        }
    }
    fn set_capture_active(&mut self, active: bool) -> io::Result<()> {
        if !self.source_available() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "original WGC source closed",
            ));
        }
        self.recovery.request(active);
        if !active {
            self.stop()?;
        } else {
            self.recovery
                .activate(self.live.is_some(), || self.restart(None))?;
        }
        Ok(())
    }
    fn reopen(&mut self) -> io::Result<()> {
        self.restart(None)
    }
}
impl Drop for WgcCapture {
    fn drop(&mut self) {
        if let Some(live) = self.live.take() {
            let _ = live.close();
        }
    }
}
