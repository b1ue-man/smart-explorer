//! Capture of a monitor with the DXGI desktop duplication (SCR-M4, U16-7):
//! the fallback where Graphics Capture is not available (before Windows 10
//! 1903) or does not start for a monitor. Windows can not be captured this
//! way, and the pointer is not part of the picture (the limit
//! `no_cursor_capture`). The device is created on the adapter that owns the
//! output, as a duplication requires.
use std::io;
use std::time::Duration;

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{ID3D11Texture2D, D3D11_TEXTURE2D_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication,
    IDXGIResource, DXGI_OUTDUPL_MOVE_RECT, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
};

use super::capture_source::{Grabber, FIRST_FRAME_WAIT};
use super::d3d_device::{self, DeviceSet, Readback};
use crate::screen::core::host_backend::SourceFrame;
use crate::screen::core::frame_regions::{FrameRegions, MoveRect};
use crate::screen::core::pixel_convert::Rect;
use windows::Win32::Foundation::RECT;
use crate::screen::core::pixel_convert::{rotate_bgra,BgraRotation,BgraImage,FrameSize};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_MODE_ROTATION,DXGI_MODE_ROTATION_UNSPECIFIED,
    DXGI_MODE_ROTATION_IDENTITY,DXGI_MODE_ROTATION_ROTATE90,DXGI_MODE_ROTATION_ROTATE180,DXGI_MODE_ROTATION_ROTATE270};
use crate::screen::core::session_state::SessionState;

/// The duplication of an output and the device it was made with.
struct Duplication {
    output:IDXGIOutput1,
    devices: DeviceSet,
    duplication: Option<IDXGIOutputDuplication>,
    size: FrameSize,
    rotation:BgraRotation,
}

/// The adapter and the output interface of the GDI output `device`
/// (`\\.\DISPLAYn`).
fn find_output(device: &str) -> windows::core::Result<(IDXGIAdapter, IDXGIOutput1)> {
    // SAFETY: plain enumeration through DXGI; every interface is owned.
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut adapter_index = 0;
        while let Ok(adapter) = factory.EnumAdapters1(adapter_index) {
            let mut output_index = 0;
            while let Ok(output) = adapter.EnumOutputs(output_index) {
                let described = output.GetDesc()?;
                let end = described
                    .DeviceName
                    .iter()
                    .position(|unit| *unit == 0)
                    .unwrap_or(described.DeviceName.len());
                if String::from_utf16_lossy(&described.DeviceName[..end]) == device {
                    let adapter: IDXGIAdapter = adapter.cast()?;
                    return Ok((adapter, output.cast()?));
                }
                output_index += 1;
            }
            adapter_index += 1;
        }
    }
    Err(windows::core::Error::from(
        windows::Win32::Foundation::E_FAIL,
    ))
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

fn duplicate(device: &str) -> windows::core::Result<Duplication> {
    let (adapter, output) = find_output(device)?;
    let devices = d3d_device::create_on(Some(&adapter))?;
    // SAFETY: the output and the device belong to the same adapter.
    let duplication = unsafe { output.DuplicateOutput(&devices.device)? };
    // SAFETY: a plain query of the duplication just made.
    let description = unsafe { duplication.GetDesc() };
    let rotation=rotation(description.Rotation).map_err(|_|windows::core::Error::from(windows::Win32::Foundation::E_INVALIDARG))?;
    let output_desc=unsafe {output.GetDesc()?};
    let rect=output_desc.DesktopCoordinates;
    let width=rect.right.checked_sub(rect.left).and_then(|n|u32::try_from(n).ok()).filter(|n|*n>0)
        .ok_or_else(||windows::core::Error::from(windows::Win32::Foundation::E_INVALIDARG))?;
    let height=rect.bottom.checked_sub(rect.top).and_then(|n|u32::try_from(n).ok()).filter(|n|*n>0)
        .ok_or_else(||windows::core::Error::from(windows::Win32::Foundation::E_INVALIDARG))?;
    let size=devices.delivered_size(width,height)
        .map_err(|_|windows::core::Error::from(windows::Win32::Foundation::E_INVALIDARG))?;
    Ok(Duplication {
        output,
        devices,
        duplication:Some(duplication),
        size,rotation,
    })
}

/// One monitor recorded with the desktop duplication.
pub(super) struct DxgiCapture {
    device: String,
    live: Duplication,
    readback: Readback,
    last: Option<BgraImage>,
    generation:u64,
}

/// Metadata from this duplication's retained output, never a new name lookup.
#[derive(Clone,Debug,PartialEq,Eq)]
pub(super) struct OutputBinding {
    pub device:String,pub monitor:usize,pub area:super::input_send::Area,
    pub rotation:i32,pub duplication_rotation:i32,pub generation:u64,pub size:FrameSize,
    pub attached:bool,
}

impl DxgiCapture {
    fn live_duplication(&self)->io::Result<&IDXGIOutputDuplication> {
        self.live.duplication.as_ref().ok_or_else(||io::Error::other("DXGI duplication unavailable"))
    }
    pub(super) fn output_binding(&self)->io::Result<OutputBinding> {
        // SAFETY: both interfaces belong to this already-open capture output.
        let output=unsafe {self.live.output.GetDesc()}.map_err(io::Error::other)?;
        let duplication=unsafe {self.live_duplication()?.GetDesc()};
        let end=output.DeviceName.iter().position(|unit|*unit==0).unwrap_or(output.DeviceName.len());
        let r=output.DesktopCoordinates;
        let area=super::input_send::Area {left:r.left,top:r.top,
            width:r.right.checked_sub(r.left).filter(|n|*n>0).ok_or_else(||io::Error::other("DXGI output width unavailable"))?,
            height:r.bottom.checked_sub(r.top).filter(|n|*n>0).ok_or_else(||io::Error::other("DXGI output height unavailable"))?};
        Ok(OutputBinding {device:String::from_utf16_lossy(&output.DeviceName[..end]),monitor:output.Monitor.0 as usize,
            area,rotation:output.Rotation.0,duplication_rotation:duplication.Rotation.0,generation:self.generation,
            size:self.live.size,attached:output.AttachedToDesktop.as_bool()})
    }
    /// Duplicates the monitor whose GDI output is `device` (`\\.\DISPLAYn`).
    pub(super) fn open(device: &str) -> windows::core::Result<Self> {
        Ok(Self {
            device: device.into(),
            live: duplicate(device)?,
            readback: Readback::default(),
            last: None,generation:1,
        })
    }

    /// The last picture again when the caller needs one, else "unchanged".
    fn again(&self, keyframe: bool) -> SourceFrame {
        match (&self.last, keyframe) {
            (Some(last), true) => SourceFrame::Frame(last.clone()),
            (Some(_), false) => SourceFrame::Unchanged,
            (None, _) => SourceFrame::Paused(SessionState::Starting),
        }
    }

    /// DXGI metadata is optional. An error keeps full capture and pixel comparison usable.
    fn regions(&self, metadata_bytes: u32, size: FrameSize) -> Option<FrameRegions> {
        // Native move/dirty coordinates cannot describe rotated or scaled references.
        if self.live.rotation!=BgraRotation::Identity {return None;}
        let native=unsafe {self.live_duplication().ok()?.GetDesc()}.ModeDesc;
        if size!=FrameSize::new(native.Width,native.Height) {return None;}
        let bytes = metadata_bytes as usize;
        let max_rects = size.width.div_ceil(64) as usize * size.height.div_ceil(64) as usize * 2;
        if bytes == 0 || bytes > max_rects.checked_mul(std::mem::size_of::<DXGI_OUTDUPL_MOVE_RECT>())? { return None; }
        let move_count = bytes.div_ceil(std::mem::size_of::<DXGI_OUTDUPL_MOVE_RECT>());
        let dirty_count = bytes.div_ceil(std::mem::size_of::<RECT>());
        let mut moved = vec![DXGI_OUTDUPL_MOVE_RECT::default(); move_count];
        let mut dirty = vec![RECT::default(); dirty_count];
        let mut used_moves = 0u32; let mut used_dirty = 0u32;
        // SAFETY: arrays are aligned typed storage; byte capacities and returned sizes are checked.
        unsafe {
            self.live_duplication().ok()?.GetFrameMoveRects((moved.len()*std::mem::size_of::<DXGI_OUTDUPL_MOVE_RECT>()) as u32,
                moved.as_mut_ptr(), &mut used_moves).ok()?;
            self.live_duplication().ok()?.GetFrameDirtyRects((dirty.len()*std::mem::size_of::<RECT>()) as u32,
                dirty.as_mut_ptr(), &mut used_dirty).ok()?;
        }
        if used_moves as usize > moved.len()*std::mem::size_of::<DXGI_OUTDUPL_MOVE_RECT>()
            || used_dirty as usize > dirty.len()*std::mem::size_of::<RECT>()
            || used_moves as usize % std::mem::size_of::<DXGI_OUTDUPL_MOVE_RECT>() != 0
            || used_dirty as usize % std::mem::size_of::<RECT>() != 0 { return None; }
        let rect = |r: RECT| -> Option<Rect> { Some(Rect { x: u32::try_from(r.left).ok()?, y: u32::try_from(r.top).ok()?,
            width: u32::try_from(r.right.checked_sub(r.left)?).ok()?, height: u32::try_from(r.bottom.checked_sub(r.top)?).ok()? }) };
        let mut regions = FrameRegions::default();
        for moved in moved.into_iter().take(used_moves as usize/std::mem::size_of::<DXGI_OUTDUPL_MOVE_RECT>()) {
            regions.moves.push(MoveRect { source_x: u32::try_from(moved.SourcePoint.x).ok()?,
                source_y: u32::try_from(moved.SourcePoint.y).ok()?, destination: rect(moved.DestinationRect)? });
        }
        for dirty in dirty.into_iter().take(used_dirty as usize/std::mem::size_of::<RECT>()) { regions.dirty.push(rect(dirty)?); }
        regions.validate(size).ok()?; Some(regions)
    }

    fn read(&mut self, resource: &IDXGIResource) -> io::Result<BgraImage> {
        let texture: ID3D11Texture2D = resource.cast().map_err(io::Error::other)?;
        let mut description = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: a valid out structure.
        unsafe { texture.GetDesc(&mut description) };
        let current=rotation(unsafe {self.live_duplication()?.GetDesc()}.Rotation)?;
        if current!=self.live.rotation {
            self.generation=self.generation.checked_add(1).ok_or_else(||io::Error::other("DXGI reference generation exhausted"))?;
            self.live.rotation=current;
        }
        let limits=self.live.devices.capture_limits(FrameSize::new(description.Width,description.Height))?;
        let image=self.readback.read(&self.live.devices,&texture,description.Width,description.Height)?;
        rotate_bgra(image,current,limits)
    }
}

impl Grabber for DxgiCapture {
    fn source_available(&self)->bool {self.live.duplication.is_some()}
    fn reference_generation(&self)->u64 {self.generation}
    fn frame_limits(&self)->crate::screen::core::frame_budget::FrameLimits {
        let Ok(duplication)=self.live_duplication() else{return crate::screen::core::frame_budget::FrameLimits::LEGACY;};
        let desc=unsafe {duplication.GetDesc()};
        self.live.devices.capture_limits(FrameSize::new(desc.ModeDesc.Width,desc.ModeDesc.Height)).unwrap_or(crate::screen::core::frame_budget::FrameLimits::LEGACY)
    }
    fn size(&self) -> FrameSize {
        self.live.size
    }

    fn capture(&mut self, keyframe: bool) -> io::Result<SourceFrame> {
        // The first picture may take a display refresh; later calls only
        // look whether one is ready.
        let wait = if self.last.is_none() {
            FIRST_FRAME_WAIT
        } else {
            Duration::ZERO
        };
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        // SAFETY: the out pointers address locals; a frame acquired here is
        // released below before the next call.
        let acquired = unsafe {
            self.live_duplication()?.AcquireNextFrame(
                wait.as_millis() as u32,
                &mut info,
                &mut resource,
            )
        };
        match acquired {
            Ok(()) => {
                // Only the pointer moved: the desktop is as it was.
                let picture = match resource {
                    Some(resource) if info.AccumulatedFrames > 0 || self.last.is_none() => {
                        Some(self.read(&resource))
                    }
                    _ => None,
                };
                let regions = if self.last.is_some() { self.regions(info.TotalMetadataBufferSize,self.live.size) } else { None };
                // SAFETY: the frame was acquired above.
                let _ = unsafe { self.live_duplication()?.ReleaseFrame() };
                match picture {
                    Some(image) => {
                        let mut image = image?;
                        if let Some(regions) = regions { let _ = image.set_regions(regions); }
                        self.live.size = image.size();
                        self.last = Some(image.clone());
                        Ok(SourceFrame::Frame(image))
                    }
                    None => Ok(self.again(keyframe)),
                }
            }
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => Ok(self.again(keyframe)),
            Err(error) if error.code() == DXGI_ERROR_ACCESS_LOST => {
                drop(resource);
                self.reopen()?;
                Ok(SourceFrame::Paused(SessionState::Starting))
            }
            Err(error) => Err(io::Error::other(error)),
        }
    }

    fn reopen(&mut self) -> io::Result<()> {
        // DuplicateOutput must not overlap the old duplication, even when the
        // new creation fails. No stale surface or move/dirty history survives.
        drop(self.live.duplication.take());
        self.readback.reset();self.last=None;
        self.generation=self.generation.checked_add(1).ok_or_else(||io::Error::other("DXGI reference generation exhausted"))?;
        self.live=duplicate(&self.device).map_err(io::Error::other)?;
        Ok(())
    }
}
