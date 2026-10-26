//! Windows Graphics Capture: metadata and an exact grant precede pool creation.
//! Cached and returned pictures are aliases of immutable admitted storage.
use std::io;
use std::sync::{Arc,OnceLock};
use std::sync::atomic::{AtomicBool,Ordering};
use std::time::{Duration,Instant};
use windows::core::Interface;
use windows::Foundation::{EventRegistrationToken,TypedEventHandler};
use windows::Graphics::Capture::{Direct3D11CaptureFrame,GraphicsCaptureItem,GraphicsCaptureSession};
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use crate::screen::core::capture_admission::CaptureGrant;
use crate::screen::core::capture_image::CapturedImage;
use crate::screen::core::host_backend::SourceFrame;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::session_state::SessionState;
use super::capture_slots::{CaptureBudget,CaptureGeometry,CaptureKind};
use super::capture_source::{Grabber,FIRST_FRAME_WAIT};
use super::capture_retirement::CaptureResources;
use super::d3d_device::{self,DeviceSet,Readback};
use super::monitors;
const POLL:Duration=Duration::from_millis(5);
#[cfg(debug_assertions)]
static FORCED_OFF:AtomicBool=AtomicBool::new(false);
#[cfg(debug_assertions)]
pub(super) fn force_unavailable_for_tests(off:bool) {FORCED_OFF.store(off,Ordering::SeqCst);}
pub(super) fn available()->bool {
    #[cfg(debug_assertions)]
    if FORCED_OFF.load(Ordering::SeqCst) {return false;}
    static AVAILABLE:OnceLock<bool>=OnceLock::new();
    *AVAILABLE.get_or_init(|| GraphicsCaptureSession::IsSupported().unwrap_or(false)
        && windows::core::factory::<GraphicsCaptureItem,IGraphicsCaptureItemInterop>().is_ok())
}
#[derive(Clone,Debug,PartialEq,Eq)]
pub(super) enum Target {Monitor(String),Window(usize)}

/// CreateForWindow/Monitor supplies only a retained item and metadata. No
/// framepool or capture session exists until activate consumes its grant.
pub(super) struct PreparedWgc {
    devices:DeviceSet,
    item:GraphicsCaptureItem,
    closed:Arc<AtomicBool>,
    closed_token:EventRegistrationToken,
}
impl PreparedWgc {
    pub(super) fn prepare(target:&Target)->io::Result<Self> {
        let interop=windows::core::factory::<GraphicsCaptureItem,IGraphicsCaptureItemInterop>().map_err(io::Error::other)?;
        let item:GraphicsCaptureItem=match target {
            Target::Monitor(id)=>{
                let entry=monitors::find(id).ok_or_else(||io::Error::new(io::ErrorKind::NotFound,"capture monitor disappeared"))?;
                // SAFETY: the monitor was just enumerated; the item retains this target.
                unsafe {interop.CreateForMonitor(entry.handle)}.map_err(io::Error::other)?
            }
            // SAFETY: invalid window handles cause an error; no desktop fallback.
            Target::Window(handle)=>unsafe {interop.CreateForWindow(HWND(*handle as *mut _))}.map_err(io::Error::other)?,
        };
        let closed=Arc::new(AtomicBool::new(false));
        let flag=closed.clone();
        let token=item.Closed(&TypedEventHandler::<GraphicsCaptureItem,windows::core::IInspectable>::new(move |_,_|{
            flag.store(true,Ordering::Release);Ok(())
        })).map_err(io::Error::other)?;
        let devices=match d3d_device::create() {Ok(devices)=>devices,Err(error)=>{
            let _=item.RemoveClosed(token);return Err(io::Error::other(error));
        }};
        Ok(Self {devices,item,closed,closed_token:token})
    }
    pub(super) fn geometry(&self)->io::Result<Option<CaptureGeometry>> {
        if self.closed.load(Ordering::Acquire) {return Err(io::Error::new(io::ErrorKind::NotFound,"original WGC source closed"));}
        let size=self.item.Size().map_err(io::Error::other)?;
        if size.Width<=0 || size.Height<=0 {return Ok(None);}
        let native=FrameSize::new(size.Width as u32,size.Height as u32);
        let limits=self.devices.capture_limits(native)?;
        Ok(Some(CaptureGeometry {native,delivered:self.devices.delivered_size(native.width,native.height)?,limits,kind:CaptureKind::Wgc}))
    }
}
impl Drop for PreparedWgc {fn drop(&mut self) {let _=self.item.RemoveClosed(self.closed_token);}}
struct Live {
    resources:CaptureResources,
    size:FrameSize,
}
impl Live {
    fn close(self)->io::Result<()> {
        self.resources.close()
    }
}
fn create(target:&PreparedWgc,budget:&CaptureBudget)->io::Result<Live> {
    let expected=budget.geometry().native;
    let actual=target.geometry()?.ok_or_else(||io::Error::other("WGC native metadata pending"))?;
    if actual.native!=expected {return Err(io::Error::other("WGC geometry changed before activation"));}
    let slice=budget.claim_native()?;
    let size=SizeInt32 {Width:expected.width as i32,Height:expected.height as i32};
    let resources=CaptureResources::create(&target.devices.winrt,&target.item,size,slice)?;
    Ok(Live {resources,size:expected})
}
pub(super) struct WgcCapture {
    live:Option<Live>,
    target:PreparedWgc,
    readback:Readback,
    last:Option<CapturedImage>,
    size:FrameSize,
    budget:CaptureBudget,
}
impl WgcCapture {
    pub(super) fn open(target:PreparedWgc,budget:CaptureBudget)->io::Result<Self> {
        let live=create(&target,&budget)?;
        let size=budget.geometry().displayed();
        Ok(Self {live:Some(live),target,readback:Readback::default(),last:None,size,budget})
    }
    fn newest(&self)->io::Result<Option<Direct3D11CaptureFrame>> {
        let mut newest:Option<Direct3D11CaptureFrame>=None;
        loop {
            let pool=self.live.as_ref().ok_or_else(||io::Error::other("WGC capture paused"))?.resources.pool()?;
            match pool.TryGetNextFrame() {
                Ok(frame)=>{if let Some(older)=newest.replace(frame) {let _=older.Close();}}
                Err(error) if error.code().is_ok()=>return Ok(newest),
                Err(error)=>return newest.map(Some).ok_or_else(||io::Error::other(error)),
            }
        }
    }
    fn restart(&mut self)->io::Result<()> {
        self.stop()?;
        let geometry=self.target.geometry()?.ok_or_else(||io::Error::other("WGC native metadata pending"))?;
        self.budget.replace(geometry)?;
        self.live=Some(create(&self.target,&self.budget)?);
        self.size=geometry.displayed();Ok(())
    }
    fn stop(&mut self)->io::Result<()> {
        if let Some(live)=self.live.take() {
            if let Err(error)=live.close() {self.target.closed.store(true,Ordering::Release);return Err(error);}
        }
        self.readback.reset();self.last=None;self.budget.suspend();Ok(())
    }
    fn read(&mut self,frame:&Direct3D11CaptureFrame)->io::Result<Option<CapturedImage>> {
        let content=frame.ContentSize().map_err(io::Error::other)?;
        if content.Width<=0 || content.Height<=0 {return Err(io::Error::other("capture frame has no content"));}
        let size=FrameSize::new(content.Width as u32,content.Height as u32);
        if self.live.as_ref().map(|live|live.size)!=Some(size) {return Ok(None);}
        let surface=frame.Surface().map_err(io::Error::other)?;
        let access:IDirect3DDxgiInterfaceAccess=surface.cast().map_err(io::Error::other)?;
        // SAFETY: a WGC surface is a D3D11 texture of this frame's device.
        let texture:ID3D11Texture2D=unsafe {access.GetInterface()}.map_err(io::Error::other)?;
        Ok(Some(self.readback.read(&self.target.devices,&texture,size.width,size.height,&self.budget)?.freeze()))
    }
}
impl Grabber for WgcCapture {
    fn frame_limits(&self)->crate::screen::core::frame_budget::FrameLimits {self.budget.geometry().limits}
    fn capture_grant(&self)->Option<CaptureGrant> {self.budget.grant().ok().cloned()}
    fn reference_generation(&self)->u64 {self.budget.generation()}
    fn source_available(&self)->bool {!self.target.closed.load(Ordering::Acquire)}
    fn size(&self)->FrameSize {self.size}
    fn capture(&mut self,keyframe:bool)->io::Result<SourceFrame> {
        if !self.source_available() {return Err(io::Error::new(io::ErrorKind::NotFound,"WGC source closed"));}
        if self.live.is_none() {return Ok(SourceFrame::Unchanged);}
        let deadline=Instant::now()+FIRST_FRAME_WAIT;
        loop {
            if let Some(frame)=self.newest()? {
                // A retained external result has its own immutable owner;
                // the source cache must not force a third distinct CPU slot.
                self.last=None;
                let image=self.read(&frame);
                let closed=frame.Close();drop(frame);
                closed.map_err(io::Error::other)?;
                // Pool replacement occurs only after returning the acquired
                // frame, closing the old pool and obtaining the next grant.
                let image=match image {Err(error) if error.kind()==io::ErrorKind::WouldBlock=>return Ok(SourceFrame::Unchanged), result=>result?};
                let Some(image)=image else {self.restart()?;continue;};
                self.size=image.size();self.last=Some(image.clone());return Ok(SourceFrame::Frame(image));
            }
            if let Some(last)=&self.last {return Ok(if keyframe {SourceFrame::Frame(last.clone())} else {SourceFrame::Unchanged});}
            if Instant::now()>=deadline {return Ok(SourceFrame::Paused(SessionState::Starting));}
            std::thread::sleep(POLL);
        }
    }
    fn set_capture_active(&mut self,active:bool)->io::Result<()> {
        if !self.source_available() {return Err(io::Error::new(io::ErrorKind::NotFound,"original WGC source closed"));}
        if !active {self.stop()?;} else if self.live.is_none() {self.restart()?;}
        Ok(())
    }
    fn reopen(&mut self)->io::Result<()> {self.restart()}
}
impl Drop for WgcCapture {fn drop(&mut self) {if let Some(live)=self.live.take() {let _=live.close();}}}
