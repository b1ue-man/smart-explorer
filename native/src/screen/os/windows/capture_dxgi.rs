//! Retained-output DXGI duplication. Prepare enumerates metadata; activation,
//! reopen and mode changes acquire an exact grant before DuplicateOutput.
use std::io;
use std::time::Duration;
use windows::core::Interface;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{ID3D11Texture2D,D3D11_TEXTURE2D_DESC};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1,IDXGIAdapter,IDXGIFactory1,IDXGIOutput1,
    IDXGIOutputDuplication,IDXGIResource,DXGI_OUTDUPL_MOVE_RECT,DXGI_OUTDUPL_FRAME_INFO,
    DXGI_ERROR_ACCESS_LOST,DXGI_ERROR_WAIT_TIMEOUT};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_MODE_ROTATION,DXGI_MODE_ROTATION_UNSPECIFIED,
    DXGI_MODE_ROTATION_IDENTITY,DXGI_MODE_ROTATION_ROTATE90,DXGI_MODE_ROTATION_ROTATE180,DXGI_MODE_ROTATION_ROTATE270};
use crate::screen::core::capture_admission::{CaptureGrant,CaptureOwner,CaptureSlice};
use crate::screen::core::capture_image::{AdmittedRegions,CapturedImage,CaptureImageBuilder};
use crate::screen::core::frame_regions::MoveRect;
use crate::screen::core::host_backend::SourceFrame;
use crate::screen::core::pixel_convert::{BgraRotation,FrameSize,Rect};
use crate::screen::core::session_state::SessionState;
use super::capture_slots::{CaptureBudget,CaptureGeometry,CaptureKind};
use super::capture_source::{Grabber,FIRST_FRAME_WAIT};
use super::capture_prepared::recovery::CaptureRecovery;
use super::d3d_device::{self,DeviceSet,Readback};

fn find_output(device:&str)->io::Result<(IDXGIAdapter,IDXGIOutput1)> {
    // SAFETY: plain owned DXGI enumeration; no capture surface is opened.
    unsafe {
        let factory:IDXGIFactory1=CreateDXGIFactory1().map_err(io::Error::other)?;
        let mut adapter_index=0;
        while let Ok(adapter)=factory.EnumAdapters1(adapter_index) {
            let mut output_index=0;
            while let Ok(output)=adapter.EnumOutputs(output_index) {
                let desc=output.GetDesc().map_err(io::Error::other)?;
                let end=desc.DeviceName.iter().position(|u|*u==0).unwrap_or(desc.DeviceName.len());
                if String::from_utf16_lossy(&desc.DeviceName[..end])==device {
                    return Ok((adapter.cast().map_err(io::Error::other)?,output.cast().map_err(io::Error::other)?));
                }
                output_index+=1;
            }
            adapter_index+=1;
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound,"original DXGI output unavailable"))
}
fn rotation(value:DXGI_MODE_ROTATION)->io::Result<BgraRotation> {
    match value {
        DXGI_MODE_ROTATION_UNSPECIFIED|DXGI_MODE_ROTATION_IDENTITY=>Ok(BgraRotation::Identity),
        DXGI_MODE_ROTATION_ROTATE90=>Ok(BgraRotation::Clockwise90),
        DXGI_MODE_ROTATION_ROTATE180=>Ok(BgraRotation::Clockwise180),
        DXGI_MODE_ROTATION_ROTATE270=>Ok(BgraRotation::Clockwise270),
        _=>Err(io::Error::new(io::ErrorKind::Unsupported,"unknown DXGI desktop rotation")),
    }
}
pub(super) struct PreparedDxgi {
    output:IDXGIOutput1,
    devices:DeviceSet,
    device:String,
    monitor:usize,
}
impl PreparedDxgi {
    pub(super) fn prepare(device:&str)->io::Result<Self> {
        let (adapter,output)=find_output(device)?;
        // Device creation has no requested capture pixels. The actual feature
        // level, queried here, determines the metadata-only texture limits.
        let devices=d3d_device::create_on(Some(&adapter)).map_err(io::Error::other)?;
        let monitor=unsafe {output.GetDesc()}.map_err(io::Error::other)?.Monitor.0 as usize;
        let prepared=Self {output,devices,device:device.into(),monitor};
        prepared.geometry()?;Ok(prepared)
    }
    pub(super) fn monitor(&self)->usize {self.monitor}
    pub(super) fn geometry(&self)->io::Result<CaptureGeometry> {
        // SAFETY: immutable query on the retained output, never a name rebind.
        let desc=unsafe {self.output.GetDesc()}.map_err(io::Error::other)?;
        let end=desc.DeviceName.iter().position(|u|*u==0).unwrap_or(desc.DeviceName.len());
        if !desc.AttachedToDesktop.as_bool() || desc.Monitor.0 as usize!=self.monitor
            || String::from_utf16_lossy(&desc.DeviceName[..end])!=self.device {
            return Err(io::Error::new(io::ErrorKind::NotFound,"original DXGI output changed"));
        }
        let r=desc.DesktopCoordinates;
        let width=r.right.checked_sub(r.left).and_then(|n|u32::try_from(n).ok()).filter(|n|*n>0)
            .ok_or_else(||io::Error::other("DXGI output width unavailable"))?;
        let height=r.bottom.checked_sub(r.top).and_then(|n|u32::try_from(n).ok()).filter(|n|*n>0)
            .ok_or_else(||io::Error::other("DXGI output height unavailable"))?;
        let rotation=rotation(desc.Rotation)?;
        let native=match rotation {BgraRotation::Clockwise90|BgraRotation::Clockwise270=>FrameSize::new(height,width),
            _=>FrameSize::new(width,height)};
        let limits=self.devices.capture_limits(native)?;
        Ok(CaptureGeometry {native,delivered:self.devices.delivered_size(native.width,native.height)?,limits,kind:CaptureKind::Dxgi(rotation)})
    }
}
struct Live {duplication:IDXGIOutputDuplication,_native:CaptureSlice}
fn duplicate(prepared:&PreparedDxgi,budget:&CaptureBudget)->io::Result<Live> {
    let metadata=prepared.geometry()?;
    if metadata.native!=budget.geometry().native || metadata.kind!=budget.geometry().kind {
        return Err(io::Error::other("DXGI mode changed before activation"));
    }
    let slice=budget.claim_native()?;
    // SAFETY: the prepared device is on this retained output's adapter.
    let duplication=unsafe {prepared.output.DuplicateOutput(&prepared.devices.device)}.map_err(io::Error::other)?;
    let desc=unsafe {duplication.GetDesc()};
    if FrameSize::new(desc.ModeDesc.Width,desc.ModeDesc.Height)!=metadata.native
        || rotation(desc.Rotation)?!=match metadata.kind {CaptureKind::Dxgi(r)=>r,_=>BgraRotation::Identity} {
        return Err(io::Error::other("DXGI surface metadata changed during activation"));
    }
    Ok(Live {duplication,_native:slice})
}
/// Geometry published to the verified login helper's existing input guard.
#[derive(Clone,Debug,PartialEq,Eq)]
pub(super) struct OutputBinding {
    pub device:String,pub monitor:usize,pub area:super::input_send::Area,
    pub rotation:i32,pub duplication_rotation:i32,pub generation:u64,pub size:FrameSize,pub attached:bool,
}
pub(super) struct DxgiCapture {
    live:Option<Live>,
    prepared:PreparedDxgi,
    readback:Readback,
    last:Option<CapturedImage>,
    size:FrameSize,
    budget:CaptureBudget,
    recovery:CaptureRecovery,
}
impl DxgiCapture {
    pub(super) fn open(prepared:PreparedDxgi,budget:CaptureBudget)->io::Result<Self> {
        let live=duplicate(&prepared,&budget)?;
        let size=budget.geometry().displayed();
        Ok(Self {live:Some(live),prepared,readback:Readback::default(),last:None,size,budget,recovery:CaptureRecovery::active()})
    }
    fn live_duplication(&self)->io::Result<&IDXGIOutputDuplication> {
        self.live.as_ref().map(|live|&live.duplication).ok_or_else(||io::Error::other("DXGI duplication unavailable"))
    }
    pub(super) fn output_binding(&self)->io::Result<OutputBinding> {
        let output=unsafe {self.prepared.output.GetDesc()}.map_err(io::Error::other)?;
        let duplication=unsafe {self.live_duplication()?.GetDesc()};
        let end=output.DeviceName.iter().position(|u|*u==0).unwrap_or(output.DeviceName.len());
        let r=output.DesktopCoordinates;
        let area=super::input_send::Area {left:r.left,top:r.top,
            width:r.right.checked_sub(r.left).filter(|n|*n>0).ok_or_else(||io::Error::other("DXGI output width unavailable"))?,
            height:r.bottom.checked_sub(r.top).filter(|n|*n>0).ok_or_else(||io::Error::other("DXGI output height unavailable"))?};
        Ok(OutputBinding {device:String::from_utf16_lossy(&output.DeviceName[..end]),monitor:output.Monitor.0 as usize,
            area,rotation:output.Rotation.0,duplication_rotation:duplication.Rotation.0,generation:self.budget.generation(),
            size:self.size,attached:output.AttachedToDesktop.as_bool()})
    }
    fn again(&self,keyframe:bool)->SourceFrame {
        match (&self.last,keyframe) {(Some(last),true)=>SourceFrame::Frame(last.clone()),
            (Some(_),false)=>SourceFrame::Unchanged,(None,_)=>SourceFrame::Paused(SessionState::Starting)}
    }
    /// Optional native metadata may fail independently of full pixel capture.
    fn regions(&self,metadata_bytes:u32,size:FrameSize)->Option<AdmittedRegions> {
        if self.budget.geometry().kind!=CaptureKind::Dxgi(BgraRotation::Identity)
            || self.budget.geometry().native!=size || metadata_bytes==0 {return None;}
        let count=u64::from(size.width.div_ceil(64))*u64::from(size.height.div_ceil(64))*2;
        if u64::from(metadata_bytes)>count.checked_mul(std::mem::size_of::<DXGI_OUTDUPL_MOVE_RECT>() as u64)? {return None;}
        let grant=self.budget.grant().ok()?;
        let mut moved=NativeRects::<DXGI_OUTDUPL_MOVE_RECT>::new(count,grant).ok()?;
        let mut dirty=NativeRects::<RECT>::new(count,grant).ok()?;
        let mut used_moves=0;let mut used_dirty=0;
        // SAFETY: admitted aligned typed buffers; all returned lengths are
        // validated before indexing or constructing immutable core metadata.
        unsafe {
            self.live_duplication().ok()?.GetFrameMoveRects(moved.bytes,moved.values.as_mut_ptr(),&mut used_moves).ok()?;
            self.live_duplication().ok()?.GetFrameDirtyRects(dirty.bytes,dirty.values.as_mut_ptr(),&mut used_dirty).ok()?;
        }
        let moves=moved.used(used_moves)?;let dirt=dirty.used(used_dirty)?;
        let mut result=AdmittedRegions::new(size,grant,moves.len() as u64,dirt.len() as u64).ok()?;
        for moved in moves {
            result.push_move(MoveRect {source_x:u32::try_from(moved.SourcePoint.x).ok()?,
                source_y:u32::try_from(moved.SourcePoint.y).ok()?,destination:rect(moved.DestinationRect)?}).ok()?;
        }
        for dirty in dirt {result.push_dirty(rect(*dirty)?).ok()?;}
        Some(result)
    }
    fn read(&mut self,resource:&IDXGIResource)->io::Result<Option<CaptureImageBuilder>> {
        let texture:ID3D11Texture2D=resource.cast().map_err(io::Error::other)?;
        let mut desc=D3D11_TEXTURE2D_DESC::default();unsafe {texture.GetDesc(&mut desc)};
        let rotation=rotation(unsafe {self.live_duplication()?.GetDesc()}.Rotation)?;
        if FrameSize::new(desc.Width,desc.Height)!=self.budget.geometry().native
            || self.budget.geometry().kind!=CaptureKind::Dxgi(rotation) {return Ok(None);}
        self.readback.read(&self.prepared.devices,&texture,desc.Width,desc.Height,&self.budget).map(Some)
    }
    fn stop(&mut self) {
        drop(self.live.take());self.readback.reset();self.last=None;self.budget.suspend();
    }
}
impl Grabber for DxgiCapture {
    fn capture_grant(&self)->Option<CaptureGrant> {self.budget.grant().ok().cloned()}
    fn source_available(&self)->bool {self.prepared.geometry().is_ok()}
    fn reference_generation(&self)->u64 {self.budget.generation()}
    fn frame_limits(&self)->crate::screen::core::frame_budget::FrameLimits {self.budget.geometry().limits}
    fn size(&self)->FrameSize {self.size}
    fn capture(&mut self,keyframe:bool)->io::Result<SourceFrame> {
        let recovery=self.recovery;
        if !recovery.ensure_live(self.live.is_some(),||self.reopen())? {return Ok(SourceFrame::Unchanged);}
        let wait=if self.last.is_none() {FIRST_FRAME_WAIT} else {Duration::ZERO};
        let mut info=DXGI_OUTDUPL_FRAME_INFO::default();let mut resource=None;
        let acquired=unsafe {self.live_duplication()?.AcquireNextFrame(wait.as_millis() as u32,&mut info,&mut resource)};
        match acquired {
            Ok(())=>{
                let had_reference=self.last.is_some();
                let picture=match resource.as_ref() {
                    Some(resource) if info.AccumulatedFrames>0||!had_reference=>{self.last=None;Some(self.read(resource))},_=>None,
                };
                let regions=if had_reference {self.regions(info.TotalMetadataBufferSize,self.size)} else {None};
                drop(resource);
                let released=unsafe {self.live_duplication()?.ReleaseFrame()};
                if let Err(error)=released {
                    drop(picture);drop(regions);
                    if error.code()==DXGI_ERROR_ACCESS_LOST {
                        self.reopen()?;return Ok(SourceFrame::Paused(SessionState::Starting));
                    }
                    return Err(io::Error::other(error));
                }
                match picture {
                    Some(Ok(Some(mut builder)))=>{
                        if let Some(regions)=regions {builder.set_regions(regions)?;}
                        let image=builder.freeze();
                        let rotation=match self.budget.geometry().kind {CaptureKind::Dxgi(r)=>r,_=>BgraRotation::Identity};
                        let image=match image.rotate(rotation,self.budget.geometry().limits,self.budget.grant()?,CaptureOwner::SourcePixels) {
                            Err(error) if error.kind()==io::ErrorKind::WouldBlock=>return Ok(SourceFrame::Unchanged),
                            result=>result?,
                        };
                        self.size=image.size();self.last=Some(image.clone());Ok(SourceFrame::Frame(image))
                    }
                    Some(Ok(None))=>{self.reopen()?;Ok(SourceFrame::Paused(SessionState::Starting))}
                    Some(Err(error)) if error.kind()==io::ErrorKind::WouldBlock=>Ok(SourceFrame::Unchanged),
                    Some(Err(error))=>Err(error),None=>Ok(self.again(keyframe)),
                }
            }
            Err(error) if error.code()==DXGI_ERROR_WAIT_TIMEOUT=>Ok(self.again(keyframe)),
            Err(error) if error.code()==DXGI_ERROR_ACCESS_LOST=>{drop(resource);self.reopen()?;Ok(SourceFrame::Paused(SessionState::Starting))}
            Err(error)=>Err(io::Error::other(error)),
        }
    }
    fn reopen(&mut self)->io::Result<()> {
        self.recovery.request(true);
        self.stop();
        let geometry=self.prepared.geometry()?;
        self.budget.replace(geometry)?;
        self.live=Some(duplicate(&self.prepared,&self.budget)?);
        self.size=geometry.displayed();Ok(())
    }
    fn set_capture_active(&mut self,active:bool)->io::Result<()> {
        self.recovery.request(active);
        if !active {self.stop();} else {self.recovery.activate(self.live.is_some(),||self.reopen())?;}
        Ok(())
    }
}
fn rect(r:RECT)->Option<Rect> {
    Some(Rect {x:u32::try_from(r.left).ok()?,y:u32::try_from(r.top).ok()?,
        width:u32::try_from(r.right.checked_sub(r.left)?).ok()?,height:u32::try_from(r.bottom.checked_sub(r.top)?).ok()?})
}
struct NativeRects<T> {values:Vec<T>,bytes:u32,_slice:CaptureSlice}
impl<T:Default+Clone> NativeRects<T> {
    fn new(count:u64,grant:&CaptureGrant)->io::Result<Self> {
        let bytes=count.checked_mul(std::mem::size_of::<T>() as u64).ok_or_else(||io::Error::other("DXGI metadata overflow"))?;
        let length=usize::try_from(count).map_err(|_|io::Error::other("DXGI metadata count exceeds address space"))?;
        let capacity=u32::try_from(bytes).map_err(|_|io::Error::other("DXGI metadata exceeds API capacity"))?;
        let slice=grant.claim(CaptureOwner::Metadata,bytes)?;
        let mut values=Vec::new();values.try_reserve_exact(length).map_err(|_|io::Error::other("DXGI metadata allocation failed"))?;
        values.resize(length,T::default());Ok(Self {values,bytes:capacity,_slice:slice})
    }
    fn used(&self,bytes:u32)->Option<&[T]> {
        let length=bytes as usize;
        if bytes>self.bytes || length%std::mem::size_of::<T>()!=0 {return None;}
        self.values.get(..length/std::mem::size_of::<T>())
    }
}
