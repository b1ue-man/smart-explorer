//! Capture with Windows Graphics Capture (SCR-M4, U16-7 and U16-9): a
//! monitor or a window as a source, from Windows 10 1903 on. The capture
//! item comes from the interop factory of the capture item class
//! (`CreateForMonitor`, `CreateForWindow`) and not from the WinRT factories
//! with package rights, which need an app identity (NR N24). The frame pool
//! is free-threaded; `capture` collects the pictures that arrived since the
//! last call without waiting for an event, keeps the newest and closes the
//! older. Every picture is read back to the CPU (`d3d_device::Readback`).
use std::io;
use std::sync::{OnceLock,Arc};
use std::sync::atomic::{AtomicBool,Ordering};
use windows::Foundation::{TypedEventHandler,EventRegistrationToken};
use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

use super::capture_source::{Grabber, FIRST_FRAME_WAIT};
use super::d3d_device::{DeviceSet, Readback};
use super::monitors;
use crate::screen::core::host_backend::SourceFrame;
use crate::screen::core::pixel_convert::{BgraImage, FrameSize};
use crate::screen::core::session_state::SessionState;

/// Pictures the frame pool holds.
const POOL_BUFFERS: i32 = 2;
/// Wait between two looks at the pool while the first picture is awaited.
const POLL: Duration = Duration::from_millis(5);
/// Size of the pool when the item reports none (a minimized window); the
/// first real picture recreates the pool at its size.
const FALLBACK_SIZE: SizeInt32 = SizeInt32 {
    Width: 1280,
    Height: 720,
};
const FORMAT: DirectXPixelFormat = DirectXPixelFormat::B8G8R8A8UIntNormalized;

#[cfg(debug_assertions)]
static FORCED_OFF: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Development builds only: makes `available` answer `false`, so the DXGI
/// fallback can be exercised on a machine that has Graphics Capture.
#[cfg(debug_assertions)]
pub(super) fn force_unavailable_for_tests(off: bool) {
    FORCED_OFF.store(off, std::sync::atomic::Ordering::SeqCst);
}

/// Whether Graphics Capture works here: the system supports it and the
/// interop factory answers. Asked once per process; no version check.
pub(super) fn available() -> bool {
    #[cfg(debug_assertions)]
    if FORCED_OFF.load(std::sync::atomic::Ordering::SeqCst) {
        return false;
    }
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        GraphicsCaptureSession::IsSupported().unwrap_or(false)
            && windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().is_ok()
    })
}

/// What Graphics Capture records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Target {
    /// The monitor with this id (`DISPLAYn`).
    Monitor(String),
    /// The window with this handle value.
    Window(usize),
}

struct Live {
    /// Kept for the life of the session.
    _item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    pool_size: SizeInt32,
}

impl Live {
    fn close(&self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

fn create(devices: &DeviceSet, target: &Target) -> windows::core::Result<Live> {
    let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    let item: GraphicsCaptureItem = match target {
        Target::Monitor(id) => {
            let entry = monitors::find(id)
                .ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?;
            // SAFETY: a monitor handle just enumerated.
            unsafe { interop.CreateForMonitor(entry.handle)? }
        }
        // SAFETY: a window handle value; a window that is gone makes the call fail.
        Target::Window(hwnd) => unsafe { interop.CreateForWindow(HWND(*hwnd as *mut _))? },
    };
    create_for_item(devices,item)
}

fn create_for_item(devices: &DeviceSet,item: GraphicsCaptureItem) -> windows::core::Result<Live> {
    let reported = item.Size()?;
    let pool_size = if reported.Width > 0 && reported.Height > 0 {
        reported
    } else {
        FALLBACK_SIZE
    };
    devices.capture_limits(FrameSize::new(pool_size.Width as u32,pool_size.Height as u32))
        .map_err(|_|windows::core::Error::from(windows::Win32::Foundation::E_INVALIDARG))?;
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &devices.winrt,
        FORMAT,
        POOL_BUFFERS,
        pool_size,
    )?;
    let session = match pool.CreateCaptureSession(&item) {Ok(session)=>session,Err(error)=>{let _=pool.Close();return Err(error);}};
    // Older builds always include the pointer.
    let _ = session.SetIsCursorCaptureEnabled(true);
    if let Err(error)=session.StartCapture() {let _=session.Close();let _=pool.Close();return Err(error);}
    Ok(Live {
        _item: item,
        pool,
        session,
        pool_size,
    })
}

/// One monitor or window recorded with Graphics Capture.
pub(super) struct WgcCapture {
    devices: DeviceSet,
    live: Option<Live>,
    item: GraphicsCaptureItem,
    closed: Arc<AtomicBool>,
    closed_token: EventRegistrationToken,
    readback: Readback,
    last: Option<BgraImage>,
    size: FrameSize,
}

impl WgcCapture {
    pub(super) fn open(devices: DeviceSet, target: Target) -> windows::core::Result<Self> {
        let live = create(&devices, &target)?;
        let size=devices.delivered_size(live.pool_size.Width as u32,live.pool_size.Height as u32)
            .map_err(|_|windows::core::Error::from(windows::Win32::Foundation::E_INVALIDARG))?;
        let item=live._item.clone(); let closed=Arc::new(AtomicBool::new(false));
        let flag=closed.clone();
        let closed_token=match item.Closed(&TypedEventHandler::<GraphicsCaptureItem,windows::core::IInspectable>::new(move |_,_|{flag.store(true,Ordering::Release);Ok(())})) {Ok(token)=>token,Err(error)=>{live.close();return Err(error);}};
        Ok(Self {
            devices,
            live: Some(live), item, closed, closed_token,
            readback: Readback::default(),
            last: None,
            size,
        })
    }

    /// The newest picture in the pool; older ones are closed.
    fn newest(&self) -> io::Result<Option<Direct3D11CaptureFrame>> {
        let mut newest: Option<Direct3D11CaptureFrame> = None;
        loop {
            let live=self.live.as_ref().ok_or_else(||io::Error::other("WGC capture is paused"))?;
            match live.pool.TryGetNextFrame() {
                Ok(frame) => {
                    if let Some(older) = newest.replace(frame) {
                        let _ = older.Close();
                    }
                }
                // No picture waits: the pool answers with an empty result.
                Err(error) if error.code().is_ok() => return Ok(newest),
                Err(error) => {
                    return match newest {
                        Some(frame) => Ok(Some(frame)),
                        None => Err(io::Error::other(error)),
                    }
                }
            }
        }
    }

    fn read(&mut self, frame: &Direct3D11CaptureFrame) -> io::Result<Option<BgraImage>> {
        let content = frame.ContentSize().map_err(io::Error::other)?;
        if content.Width <= 0 || content.Height <= 0 {
            return Err(io::Error::other("the capture frame has no content"));
        }
        let live=self.live.as_mut().ok_or_else(||io::Error::other("WGC capture is paused"))?;
        self.devices.capture_limits(FrameSize::new(content.Width as u32,content.Height as u32))?;
        if content != live.pool_size {
            live
                .pool
                .Recreate(&self.devices.winrt, FORMAT, POOL_BUFFERS, content)
                .map_err(io::Error::other)?;
            live.pool_size = content;
            self.readback.reset();
            self.last = None;
            // This frame may still belong to the old pool and contain a clipped resize.
            return Ok(None);
        }
        let surface = frame.Surface().map_err(io::Error::other)?;
        let access: IDirect3DDxgiInterfaceAccess = surface.cast().map_err(io::Error::other)?;
        // SAFETY: the surface of a capture frame is a Direct3D 11 texture.
        let texture: ID3D11Texture2D =
            unsafe { access.GetInterface() }.map_err(io::Error::other)?;
        self.readback.read(
            &self.devices,
            &texture,
            content.Width as u32,
            content.Height as u32,
        ).map(Some)
    }
}

impl Grabber for WgcCapture {
    fn frame_limits(&self)->crate::screen::core::frame_budget::FrameLimits {
        let size=self.live.as_ref().map(|live|FrameSize::new(live.pool_size.Width as u32,live.pool_size.Height as u32)).unwrap_or(self.size);
        self.devices.capture_limits(size).unwrap_or(crate::screen::core::frame_budget::FrameLimits::LEGACY)
    }
    fn source_available(&self) -> bool {!self.closed.load(Ordering::Acquire)}
    fn size(&self) -> FrameSize {
        self.size
    }

    fn capture(&mut self, keyframe: bool) -> io::Result<SourceFrame> {
        if self.closed.load(Ordering::Acquire) {return Err(io::Error::new(io::ErrorKind::NotFound,"WGC source closed"));}
        if self.live.is_none() {return Ok(SourceFrame::Unchanged);}
        let deadline = Instant::now() + FIRST_FRAME_WAIT;
        loop {
            if let Some(frame) = self.newest()? {
                let image = self.read(&frame);
                let _ = frame.Close();
                let Some(image)=image? else {continue;};
                self.size = image.size();
                self.last = Some(image.clone());
                return Ok(SourceFrame::Frame(image));
            }
            if let Some(last) = &self.last {
                return Ok(if keyframe {
                    SourceFrame::Frame(last.clone())
                } else {
                    SourceFrame::Unchanged
                });
            }
            if Instant::now() >= deadline {
                return Ok(SourceFrame::Paused(SessionState::Starting));
            }
            std::thread::sleep(POLL);
        }
    }

    fn set_capture_active(&mut self,active:bool) -> io::Result<()> {
        if self.closed.load(Ordering::Acquire) {return Err(io::Error::new(io::ErrorKind::NotFound,"original WGC source closed"));}
        if !active {
            if let Some(live)=self.live.take() {
                let session=live.session.Close(); let pool=live.pool.Close();
                if session.is_err() || pool.is_err() {self.closed.store(true,Ordering::Release);}
                session.map_err(io::Error::other)?; pool.map_err(io::Error::other)?;
            }
            self.last=None; self.readback.reset(); return Ok(());
        }
        if self.closed.load(Ordering::Acquire) {return Err(io::Error::new(io::ErrorKind::NotFound,"original WGC source closed"));}
        if self.live.is_none() {
            let live=create_for_item(&self.devices,self.item.clone()).map_err(io::Error::other)?;
            self.size=self.devices.delivered_size(live.pool_size.Width as u32,live.pool_size.Height as u32)?;
            self.live=Some(live);
        }
        Ok(())
    }
    fn reopen(&mut self) -> io::Result<()> {
        self.set_capture_active(false)?; self.set_capture_active(true)
    }

}

impl Drop for WgcCapture {
    fn drop(&mut self) {
        // Closing the session ends the capture (and the border Windows draws).
        if let Some(live)=self.live.take() {live.close();}
        let _=self.item.RemoveClosed(self.closed_token);
    }
}
