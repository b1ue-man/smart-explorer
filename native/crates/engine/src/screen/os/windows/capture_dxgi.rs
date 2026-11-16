//! Retained-output DXGI duplication. Prepare enumerates metadata; activation,
//! reopen and mode changes acquire an exact grant before DuplicateOutput.
//! Plan 2026-11-14 bildkette: first path of a monitor (B-12), waits for the
//! next due picture on the capture thread (B-10, `dxgi_wait.rs`), "unchanged"
//! from dirty and move rectangles (`dxgi_regions.rs`), pointer (B-13).
use super::capture_prepared::recovery::CaptureRecovery;
use super::capture_slots::{CaptureBudget, CaptureGeometry, CaptureKind};
use super::capture_source::{self, CapturePath, FallbackCause, Grabber, FIRST_FRAME_WAIT};
use super::d3d_device::{self, DeviceSet, Read, Readback};
use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner, CaptureSlice};
use crate::screen::core::capture_picture::{Picture, SourcePicture};
use crate::screen::core::host_backend::SourceFrame;
use crate::screen::core::pixel_convert::{BgraRotation, FrameSize};
use crate::screen::core::session_state::SessionState;
use std::io;
use std::time::{Duration, Instant};
use windows::core::{Interface, HRESULT};
use windows::Win32::Graphics::Direct3D11::{ID3D11Texture2D, D3D11_TEXTURE2D_DESC};
use windows::Win32::Graphics::Dxgi::{
    IDXGIOutput1, IDXGIOutputDuplication, IDXGIResource, DXGI_ERROR_ACCESS_LOST,
    DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
};

#[path = "dxgi_output.rs"]
mod output;
#[path = "dxgi_pointer.rs"]
pub(super) mod pointer;
#[path = "dxgi_regions.rs"]
pub(super) mod regions;
#[path = "dxgi_wait.rs"]
pub(super) mod wait;
use output::{find_output, rotation};

pub(super) struct PreparedDxgi {
    output: IDXGIOutput1,
    devices: DeviceSet,
    device: String,
    monitor: usize,
}
impl PreparedDxgi {
    pub(super) fn prepare(device: &str) -> io::Result<Self> {
        let (adapter, output) = find_output(device)?;
        // Device creation has no requested capture pixels. The actual feature
        // level, queried here, determines the metadata-only texture limits.
        let devices = d3d_device::create_on(Some(&adapter)).map_err(io::Error::other)?;
        let monitor = unsafe { output.GetDesc() }
            .map_err(io::Error::other)?
            .Monitor
            .0 as usize;
        let prepared = Self {
            output,
            devices,
            device: device.into(),
            monitor,
        };
        prepared.geometry()?;
        Ok(prepared)
    }
    pub(super) fn monitor(&self) -> usize {
        self.monitor
    }
    pub(super) fn geometry(&self) -> io::Result<CaptureGeometry> {
        // SAFETY: immutable query on the retained output, never a name rebind.
        let desc = unsafe { self.output.GetDesc() }.map_err(io::Error::other)?;
        let end = desc
            .DeviceName
            .iter()
            .position(|u| *u == 0)
            .unwrap_or(desc.DeviceName.len());
        if !desc.AttachedToDesktop.as_bool()
            || desc.Monitor.0 as usize != self.monitor
            || String::from_utf16_lossy(&desc.DeviceName[..end]) != self.device
        {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "original DXGI output changed",
            ));
        }
        let r = desc.DesktopCoordinates;
        let width = r
            .right
            .checked_sub(r.left)
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
            .ok_or_else(|| io::Error::other("DXGI output width unavailable"))?;
        let height = r
            .bottom
            .checked_sub(r.top)
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
            .ok_or_else(|| io::Error::other("DXGI output height unavailable"))?;
        let rotation = rotation(desc.Rotation)?;
        let native = match rotation {
            BgraRotation::Clockwise90 | BgraRotation::Clockwise270 => FrameSize::new(height, width),
            _ => FrameSize::new(width, height),
        };
        let limits = self.devices.capture_limits(native)?;
        Ok(CaptureGeometry {
            native,
            delivered: self.devices.delivered_size(native.width, native.height)?,
            limits,
            kind: CaptureKind::Dxgi(rotation),
        })
    }
}
struct Live {
    duplication: IDXGIOutputDuplication,
    _native: CaptureSlice,
}
fn duplicate(prepared: &PreparedDxgi, budget: &CaptureBudget) -> io::Result<Live> {
    let metadata = prepared.geometry()?;
    if metadata.native != budget.geometry().native || metadata.kind != budget.geometry().kind {
        return Err(io::Error::other("DXGI mode changed before activation"));
    }
    let slice = budget.claim_native()?;
    // SAFETY: the prepared device is on this retained output's adapter.
    let duplication = unsafe { prepared.output.DuplicateOutput(&prepared.devices.device) }
        .map_err(io::Error::other)?;
    let desc = unsafe { duplication.GetDesc() };
    if FrameSize::new(desc.ModeDesc.Width, desc.ModeDesc.Height) != metadata.native
        || rotation(desc.Rotation)?
            != match metadata.kind {
                CaptureKind::Dxgi(r) => r,
                _ => BgraRotation::Identity,
            }
    {
        return Err(io::Error::other(
            "DXGI surface metadata changed during activation",
        ));
    }
    Ok(Live {
        duplication,
        _native: slice,
    })
}
/// Geometry published to the verified login helper's existing input guard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OutputBinding {
    pub device: String,
    pub monitor: usize,
    pub area: super::input_send::Area,
    pub rotation: i32,
    pub duplication_rotation: i32,
    pub generation: u64,
    pub size: FrameSize,
    pub attached: bool,
}
pub(super) struct DxgiCapture {
    live: Option<Live>,
    prepared: PreparedDxgi,
    readback: Readback,
    /// The newest picture, CPU pixels or on the GPU (BK3b).
    last: Option<Picture>,
    size: FrameSize,
    budget: CaptureBudget,
    recovery: CaptureRecovery,
    /// Pointer for overlay and blending; the login helper captures without.
    pointer: Option<pointer::DxgiPointer>,
    /// `DXGI_ERROR_ACCESS_LOST` since the last picture.
    lost: bool,
    /// Since when, and with which result, rebuilding the duplication fails.
    failing: Option<(Instant, Option<HRESULT>)>,
}
impl DxgiCapture {
    pub(super) fn open(prepared: PreparedDxgi, budget: CaptureBudget) -> io::Result<Self> {
        let live = duplicate(&prepared, &budget)?;
        let size = budget.geometry().displayed();
        Ok(Self {
            live: Some(live),
            prepared,
            readback: Readback::default(),
            last: None,
            size,
            budget,
            recovery: CaptureRecovery::active(),
            pointer: None,
            lost: false,
            failing: None,
        })
    }
    /// Delivers the pointer to the overlay or into the pictures (B-13).
    pub(super) fn with_pointer(mut self) -> Self {
        self.pointer = Some(pointer::DxgiPointer::attach());
        self
    }
    fn note_failure(&mut self, error: &io::Error) {
        let code = capture_source::result_code(error);
        let since = self.failing.map_or_else(Instant::now, |(since, _)| since);
        self.failing = Some((since, code));
    }
    fn live_duplication(&self) -> io::Result<&IDXGIOutputDuplication> {
        self.live
            .as_ref()
            .map(|live| &live.duplication)
            .ok_or_else(|| io::Error::other("DXGI duplication unavailable"))
    }
    pub(super) fn output_binding(&self) -> io::Result<OutputBinding> {
        let output = unsafe { self.prepared.output.GetDesc() }.map_err(io::Error::other)?;
        let duplication = unsafe { self.live_duplication()?.GetDesc() };
        let end = output
            .DeviceName
            .iter()
            .position(|u| *u == 0)
            .unwrap_or(output.DeviceName.len());
        let r = output.DesktopCoordinates;
        let area = super::input_send::Area {
            left: r.left,
            top: r.top,
            width: r
                .right
                .checked_sub(r.left)
                .filter(|n| *n > 0)
                .ok_or_else(|| io::Error::other("DXGI output width unavailable"))?,
            height: r
                .bottom
                .checked_sub(r.top)
                .filter(|n| *n > 0)
                .ok_or_else(|| io::Error::other("DXGI output height unavailable"))?,
        };
        Ok(OutputBinding {
            device: String::from_utf16_lossy(&output.DeviceName[..end]),
            monitor: output.Monitor.0 as usize,
            area,
            rotation: output.Rotation.0,
            duplication_rotation: duplication.Rotation.0,
            generation: self.budget.generation(),
            size: self.size,
            attached: output.AttachedToDesktop.as_bool(),
        })
    }
    fn again(&self, keyframe: bool) -> SourcePicture {
        match (&self.last, keyframe) {
            (Some(last), true) => SourcePicture::Frame(last.clone()),
            (Some(_), false) => SourcePicture::Unchanged,
            (None, _) => SourcePicture::Paused(SessionState::Starting),
        }
    }
    fn read(&mut self, resource: &IDXGIResource) -> io::Result<Option<Read>> {
        // Handover point of BK2a (GPU path): BGRA `texture` on
        // `self.prepared.devices.device` (the device for the encoder's
        // `MFT_MESSAGE_SET_D3D_MANAGER`), valid until `ReleaseFrame` in
        // `capture_waiting` on the capture thread; BK2a copies it on the GPU.
        let texture: ID3D11Texture2D = resource.cast().map_err(io::Error::other)?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut desc) };
        let rotation = rotation(unsafe { self.live_duplication()?.GetDesc() }.Rotation)?;
        if FrameSize::new(desc.Width, desc.Height) != self.budget.geometry().native
            || self.budget.geometry().kind != CaptureKind::Dxgi(rotation)
        {
            return Ok(None);
        }
        self.readback
            .read(
                &self.prepared.devices,
                &texture,
                desc.Width,
                desc.Height,
                &self.budget,
            )
            .map(Some)
    }
    fn stop(&mut self) {
        drop(self.live.take());
        self.readback.reset();
        self.last = None;
        self.budget.suspend();
    }
    fn rebuild(&mut self) -> io::Result<()> {
        self.recovery.request(true);
        self.stop();
        let geometry = self.prepared.geometry()?;
        self.budget.replace(geometry)?;
        self.live = Some(duplicate(&self.prepared, &self.budget)?);
        self.size = geometry.displayed();
        Ok(())
    }
}
impl Grabber for DxgiCapture {
    fn capture_grant(&self) -> Option<CaptureGrant> {
        self.budget.grant().ok().cloned()
    }
    fn source_available(&self) -> bool {
        self.prepared.geometry().is_ok()
    }
    fn reference_generation(&self) -> u64 {
        self.budget.generation()
    }
    fn frame_limits(&self) -> crate::screen::core::frame_budget::FrameLimits {
        self.budget.geometry().limits
    }
    fn size(&self) -> FrameSize {
        self.size
    }
    fn path(&self) -> Option<CapturePath> {
        Some(CapturePath::DesktopDuplication)
    }
    fn fallback_cause(&self) -> Option<FallbackCause> {
        let (since, code) = self.failing?;
        capture_source::fallback_cause(code, self.lost, since.elapsed())
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
        let paused = |error| CaptureRecovery::paused_by_duplication(error).map(SourcePicture::from);
        let recovery = self.recovery;
        match recovery.ensure_live(self.live.is_some(), || self.reopen()) {
            Ok(true) => {}
            Ok(false) => return Ok(SourcePicture::Unchanged),
            Err(error) => return paused(error),
        }
        // The overlay switched: a new duplication starts with a whole picture
        // in the new state (with or without the blended pointer).
        if self.last.is_some() && self.pointer.as_ref().is_some_and(|p| p.take_switched()) {
            return match self.reopen() {
                Ok(()) => Ok(SourcePicture::Unchanged),
                Err(error) => paused(error),
            };
        }
        let duplication = self.live_duplication()?.clone();
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None;
        // SAFETY: a live duplication; the out pointers address locals.
        let acquired = unsafe {
            duplication.AcquireNextFrame(wait::timeout_ms(wait), &mut info, &mut resource)
        };
        match acquired {
            Ok(()) => {}
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                return Ok(self.again(keyframe))
            }
            Err(error) if error.code() == DXGI_ERROR_ACCESS_LOST => {
                drop(resource);
                self.lost = true;
                self.stop();
                return Ok(SourcePicture::Paused(SessionState::Starting));
            }
            Err(error) => return Err(io::Error::other(error)),
        }
        self.lost = false;
        self.failing = None;
        let had_reference = self.last.is_some();
        let geometry = self.budget.geometry();
        // Pointer and rectangles belong to this frame: read before ReleaseFrame.
        let pointer_moved = match self.pointer.as_mut() {
            Some(pointer) => pointer.update(&duplication, &info, geometry.native_displayed()),
            None => false,
        };
        let (verdict, mut regions) = if had_reference && info.AccumulatedFrames > 0 {
            regions::read(
                &duplication,
                info.TotalMetadataBufferSize,
                &self.budget,
                self.size,
            )
        } else {
            (regions::Verdict::Unknown, None)
        };
        let changed = !had_reference
            || pointer_moved
            || (info.AccumulatedFrames > 0 && verdict != regions::Verdict::Unchanged);
        if pointer_moved {
            // The regions do not name the pointer's old and new place.
            regions = None;
        }
        let picture = match resource.as_ref() {
            Some(resource) if changed => {
                if !release() {
                    regions = None;
                }
                self.last = None;
                Some(self.read(resource))
            }
            _ => None,
        };
        drop(resource);
        // SAFETY: the frame acquired above, released once.
        let released = unsafe { duplication.ReleaseFrame() };
        if let Err(error) = released {
            drop(picture);
            drop(regions);
            if error.code() == DXGI_ERROR_ACCESS_LOST {
                self.lost = true;
                self.stop();
                return Ok(SourcePicture::Paused(SessionState::Starting));
            }
            return Err(io::Error::other(error));
        }
        match picture {
            // The GPU route has the overlay and no rotation: the regions and
            // the pointer belong to CPU pixels only (BK3b).
            Some(Ok(Some(Read::Gpu(picture)))) => {
                drop(regions);
                let picture = Picture::Gpu(picture);
                self.size = picture.size();
                self.last = Some(picture.clone());
                Ok(SourcePicture::Frame(picture))
            }
            Some(Ok(Some(Read::Cpu(mut builder)))) => {
                if let Some(regions) = regions {
                    builder.set_regions(regions)?;
                }
                let rotation = match geometry.kind {
                    CaptureKind::Dxgi(r) => r,
                    _ => BgraRotation::Identity,
                };
                if let Some(pointer) = &self.pointer {
                    let delivered = builder.size();
                    pointer.blend_into(builder.pixels_mut(), delivered, geometry.native, rotation);
                }
                let image = builder.freeze();
                let image = match image.rotate(
                    rotation,
                    geometry.limits,
                    self.budget.grant()?,
                    CaptureOwner::SourcePixels,
                ) {
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        return Ok(SourcePicture::Unchanged)
                    }
                    result => result?,
                };
                self.size = image.size();
                let picture = Picture::Cpu(image);
                self.last = Some(picture.clone());
                Ok(SourcePicture::Frame(picture))
            }
            Some(Ok(None)) => {
                self.stop();
                Ok(SourcePicture::Paused(SessionState::Starting))
            }
            Some(Err(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                Ok(SourcePicture::Unchanged)
            }
            Some(Err(error)) => Err(error),
            None => Ok(self.again(keyframe)),
        }
    }
    fn reopen(&mut self) -> io::Result<()> {
        let result = self.rebuild();
        match &result {
            Ok(()) => self.failing = None,
            Err(error) => self.note_failure(error),
        }
        result
    }
    fn set_capture_active(&mut self, active: bool) -> io::Result<()> {
        self.recovery.request(active);
        if !active {
            self.stop();
        } else {
            self.recovery
                .activate(self.live.is_some(), || self.reopen())?;
        }
        Ok(())
    }
}
