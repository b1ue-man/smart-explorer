//! Retained-output DXGI duplication. Prepare enumerates metadata; activation,
//! reopen and mode changes acquire an exact grant before DuplicateOutput.
//! Plan 2026-11-14 bildkette: first path of a monitor (B-12), waits for the
//! next due picture on the capture thread (B-10, `dxgi_wait.rs`), "unchanged"
//! from dirty and move rectangles (`dxgi_regions.rs`), pointer (B-13).
//!
//! Plan 2026-11-17 P9 („dxgi failed → Rückfall“): every failure names its
//! step and result code in the host log and in the fallback detail; a
//! rebuild that fails because the retained output or its device went stale
//! (mode change, display reconfiguration, removed or reset device, a hybrid
//! system that moved the output) finds the output again and creates its
//! device anew before it asks for a fallback; capture memory still held by
//! older pictures waits instead of falling back; an `AcquireNextFrame` or
//! `ReleaseFrame` error other than a lost duplication rebuilds once before it
//! counts as a failure (it ended the capture before).
use super::capture_prepared::recovery::CaptureRecovery;
use super::capture_slots::{CaptureBudget, CaptureKind};
use super::capture_source::{self, CapturePath, FallbackCause, Grabber, FIRST_FRAME_WAIT};
use super::d3d_device::{Read, Readback};
use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner};
use crate::screen::core::capture_fallback::{FallbackDetail, FallbackStep};
use crate::screen::core::capture_picture::{Picture, SourcePicture};
use crate::screen::core::host_backend::SourceFrame;
use crate::screen::core::pixel_convert::{BgraRotation, FrameSize};
use crate::screen::core::session_state::SessionState;
use std::io;
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{ID3D11Texture2D, D3D11_TEXTURE2D_DESC};
use windows::Win32::Graphics::Dxgi::{
    IDXGIOutputDuplication, IDXGIResource, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_WAIT_TIMEOUT,
    DXGI_OUTDUPL_FRAME_INFO,
};

#[path = "dxgi_binding.rs"]
mod binding;
#[path = "dxgi_desktop.rs"]
mod desktop;
#[path = "dxgi_first_picture.rs"]
mod first_picture;
#[path = "dxgi_output.rs"]
mod output;
#[path = "dxgi_pointer.rs"]
pub(super) mod pointer;
#[path = "dxgi_prepared.rs"]
mod prepared;
#[path = "dxgi_regions.rs"]
pub(super) mod regions;
#[path = "dxgi_wait.rs"]
pub(super) mod wait;
pub(super) use binding::OutputBinding;
use output::rotation;
use prepared::{duplicate, stale, Failing, Live};
pub(super) use prepared::{DxgiFailure, PreparedDxgi};

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
    /// The way to the first picture of a duplication (`dxgi_first_picture`).
    first: first_picture::FirstPicture,
    /// Since when, in which step and with which result rebuilding fails.
    failing: Option<Failing>,
    /// P9: put the thread on the input desktop before a duplication.
    follow_desktop: bool,
    /// P9: the output was found again in this streak of failures.
    reattached: bool,
}
impl DxgiCapture {
    pub(super) fn open(prepared: PreparedDxgi, budget: CaptureBudget) -> io::Result<Self> {
        Self::open_stepped(prepared, budget).map_err(io::Error::from)
    }
    /// `open` with the failed step (P9); the failure is in the host log.
    pub(super) fn open_stepped(
        prepared: PreparedDxgi,
        budget: CaptureBudget,
    ) -> Result<Self, DxgiFailure> {
        let live = duplicate(&prepared, &budget, false)
            .inspect_err(|failure| capture_source::log_failure(failure.step, &failure.error))?;
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
            first: first_picture::FirstPicture::default(),
            failing: None,
            follow_desktop: false,
            reattached: false,
        })
    }
    /// A user session's capture follows the input desktop (P9); the login
    /// helper keeps the desktop it was bound to.
    pub(super) fn following_input_desktop(mut self) -> Self {
        self.follow_desktop = true;
        self
    }
    /// Delivers the pointer to the overlay or into the pictures (B-13).
    pub(super) fn with_pointer(mut self) -> Self {
        self.pointer = Some(pointer::DxgiPointer::attach());
        self
    }
    fn note_failure(&mut self, failure: &DxgiFailure) {
        let code = capture_source::result_code(&failure.error);
        // A streak of the same failure is logged once.
        if self
            .failing
            .is_none_or(|failing| failing.step != failure.step || failing.code != code)
        {
            capture_source::log_failure(failure.step, &failure.error);
        }
        self.failing = Some(Failing {
            since: self
                .failing
                .map_or_else(Instant::now, |failing| failing.since),
            step: failure.step,
            kind: failure.error.kind(),
            code: capture_source::result_code(&failure.error),
        });
    }
    /// An `AcquireNextFrame` or `ReleaseFrame` error: the duplication is
    /// rebuilt like a lost one; a second one before any picture counts as a
    /// failed rebuild (so the fallback can follow).
    fn frame_failed(&mut self, step: FallbackStep, error: windows::core::Error) -> SourcePicture {
        let failure = DxgiFailure {
            step,
            error: io::Error::other(error),
        };
        if self.lost {
            self.note_failure(&failure);
        } else {
            capture_source::log_failure(step, &failure.error);
        }
        self.lost = true;
        self.stop();
        SourcePicture::Paused(SessionState::Starting)
    }
    fn live_duplication(&self) -> io::Result<&IDXGIOutputDuplication> {
        self.live
            .as_ref()
            .map(|live| &live.duplication)
            .ok_or_else(|| io::Error::other("DXGI duplication unavailable"))
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
    fn rebuild(&mut self) -> Result<(), DxgiFailure> {
        self.recovery.request(true);
        self.stop();
        match self.duplicate_again() {
            Err(failure) if stale(&failure) && !self.reattached => {
                // P9: the output found again, with a device on its adapter,
                // once per streak of failures.
                self.reattached = true;
                capture_source::log_failure(failure.step, &failure.error);
                let prepared = PreparedDxgi::prepare(&self.prepared.device)
                    .map_err(DxgiFailure::at(FallbackStep::DeviceCreate))?;
                eprintln!(
                    "screen capture: output {} found again with a new device",
                    self.prepared.device
                );
                self.prepared = prepared;
                self.duplicate_again()
            }
            result => result,
        }
    }
    fn duplicate_again(&mut self) -> Result<(), DxgiFailure> {
        let geometry = self
            .prepared
            .geometry()
            .map_err(DxgiFailure::at(FallbackStep::OutputCheck))?;
        self.budget
            .replace(geometry)
            .map_err(DxgiFailure::at(FallbackStep::CaptureMemory))?;
        self.live = Some(duplicate(
            &self.prepared,
            &self.budget,
            self.follow_desktop,
        )?);
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
        let failing = self.failing?;
        capture_source::rebuild_fallback(
            failing.step,
            failing.kind,
            failing.code,
            self.lost,
            failing.since.elapsed(),
        )
    }
    fn fallback_detail(&self) -> Option<FallbackDetail> {
        self.failing.map(|failing| FallbackDetail {
            step: failing.step,
            code: failing.code.map(|code| code.0 as u32),
        })
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
                // No picture yet on a still screen: a new duplication's
                // first frame carries the whole desktop.
                if self.last.is_none() && self.first.rebuild() {
                    return match self.reopen() {
                        Ok(()) => Ok(SourcePicture::Unchanged),
                        Err(error) => paused(error),
                    };
                }
                return Ok(self.again(keyframe));
            }
            Err(error) if error.code() == DXGI_ERROR_ACCESS_LOST => {
                drop(resource);
                eprintln!(
                    "screen capture: AcquireNextFrame: duplication lost (mode or desktop change)"
                );
                self.lost = true;
                self.stop();
                return Ok(SourcePicture::Paused(SessionState::Starting));
            }
            Err(error) => {
                drop(resource);
                return Ok(self.frame_failed(FallbackStep::AcquireNextFrame, error));
            }
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
        let changed = if had_reference {
            pointer_moved || (info.AccumulatedFrames > 0 && verdict != regions::Verdict::Unchanged)
        } else {
            self.first.readable(&info)
        };
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
                eprintln!(
                    "screen capture: ReleaseFrame: duplication lost (mode or desktop change)"
                );
                self.lost = true;
                self.stop();
                return Ok(SourcePicture::Paused(SessionState::Starting));
            }
            return Ok(self.frame_failed(FallbackStep::ReleaseFrame, error));
        }
        match picture {
            // The GPU route has the overlay and no rotation: the regions and
            // the pointer belong to CPU pixels only (BK3b).
            Some(Ok(Some(Read::Gpu(picture)))) => {
                drop(regions);
                let picture = Picture::Gpu(picture);
                self.size = picture.size();
                self.last = Some(picture.clone());
                self.first.delivered();
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
                        self.first.missed();
                        return Ok(SourcePicture::Unchanged);
                    }
                    result => result?,
                };
                self.size = image.size();
                let picture = Picture::Cpu(image);
                self.last = Some(picture.clone());
                self.first.delivered();
                Ok(SourcePicture::Frame(picture))
            }
            Some(Ok(None)) => {
                self.stop();
                Ok(SourcePicture::Paused(SessionState::Starting))
            }
            Some(Err(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                self.first.missed();
                Ok(SourcePicture::Unchanged)
            }
            Some(Err(error)) => Err(error),
            None => Ok(self.again(keyframe)),
        }
    }
    fn reopen(&mut self) -> io::Result<()> {
        match self.rebuild() {
            Ok(()) => {
                self.failing = None;
                self.reattached = false;
                Ok(())
            }
            Err(failure) => {
                self.note_failure(&failure);
                Err(failure.error)
            }
        }
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
