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
#[path = "dxgi_frame_settle.rs"]
mod frame_settle;
#[path = "dxgi_output.rs"]
mod output;
#[path = "dxgi_pointer.rs"]
pub(super) mod pointer;
#[path = "dxgi_prepared.rs"]
mod prepared;
#[path = "dxgi_rebuild.rs"]
mod rebuild;
#[path = "dxgi_regions.rs"]
pub(super) mod regions;
#[path = "dxgi_wait.rs"]
pub(super) mod wait;
pub(super) use binding::OutputBinding;
use output::rotation;
use prepared::{duplicate, Failing, Live};
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
    /// Fix of 2026-11-18: the last rebuild while a refusal settles.
    tried: Option<Instant>,
    /// Measurement of 2026-11-18: rebuilds that keep failing are spaced.
    backoff: crate::screen::core::capture_lock::RebuildBackoff,
    /// Codex round 5, finding 10: since when the pointer switched while a
    /// picture shows it in the mode before (`SWITCH_WAIT`).
    switched_at: Option<Instant>,
    /// Gemini round 8, 1: whether `last` carries the blended pointer.
    last_blends: bool,
    /// Round 9 H4: the pointer mode of the picture being read, taken once.
    blends_now: bool,
    /// Round 9 H2: a frame whose read was refused for memory, still
    /// acquired (`dxgi_frame_settle.rs`).
    held: Option<IDXGIResource>,
}

/// Gemini round 8, 1: how long a read refused for want of a free picture of
/// the capture plan (`WouldBlock`: the encoder still holds one) waits for it
/// while the frame stays acquired. Before, the missed read left no reference
/// picture and the next timeout rebuilt the duplication with a new
/// generation: a new encoder about once a second on a loaded host.
const READ_WAIT: Duration = Duration::from_millis(50);

/// How long a pointer switch waits for the next frame of the duplication
/// (any, also one where only the pointer moved), which reads the pointer's
/// area anew in the new mode with the running encoder; on a screen and a
/// pointer that both stand still the duplication is opened anew after it.
const SWITCH_WAIT: Duration = Duration::from_millis(500);
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
            tried: None,
            backoff: Default::default(),
            switched_at: None,
            last_blends: false,
            blends_now: false,
            held: None,
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
            .read_with_pointer(
                &self.prepared.devices,
                &texture,
                desc.Width,
                desc.Height,
                &self.budget,
                self.pointer
                    .as_ref()
                    .and_then(|p| p.on_picture(self.blends_now)),
            )
            .map(Some)
    }
    /// `read`, waiting up to `READ_WAIT` while it is refused for memory.
    fn read_waiting(&mut self, resource: &IDXGIResource) -> io::Result<Option<Read>> {
        let until = Instant::now() + READ_WAIT;
        loop {
            match self.read(resource) {
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < until =>
                {
                    std::thread::sleep(Duration::from_millis(1));
                }
                result => return result,
            }
        }
    }
    fn stop(&mut self) {
        self.switched_at = None;
        self.held = None; // its surface goes before the duplication
        drop(self.live.take());
        self.readback.reset();
        self.last = None;
        self.budget.suspend();
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
        // The overlay switched: the next frame reads the pointer's area anew
        // in the new state (with or without the blended pointer) and keeps
        // the duplication and the encoder (Codex round 5, finding 10); only
        // without a frame for `SWITCH_WAIT` a new duplication follows.
        // Codex round 4, finding 8: without a picture none was made in the
        // mode before, so a switch that came before it needs nothing.
        let switched = self.pointer.as_ref().is_some_and(|p| p.take_switched());
        // Gemini round 8, 1: a switch back to the mode the picture shows
        // (the viewer's start toggles it twice) asks for nothing.
        let blends = self.pointer.as_ref().is_some_and(|p| p.blends());
        self.blends_now = blends;
        if switched && self.last.is_some() && blends != self.last_blends {
            self.switched_at.get_or_insert_with(Instant::now);
        } else if blends == self.last_blends {
            self.switched_at = None;
        }
        let duplication = self.live_duplication()?.clone();
        if let Some(resource) = self.held.take() {
            return self.read_held(&duplication, resource, release, keyframe);
        }
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
                let still = self
                    .switched_at
                    .is_some_and(|at| at.elapsed() >= SWITCH_WAIT);
                if (self.last.is_none() && self.first.rebuild()) || still {
                    self.switched_at = None;
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
        // A pointer switch: its area is read anew from this frame (finding 10).
        let pointer_moved =
            pointer_moved || (resource.is_some() && self.switched_at.take().is_some());
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
            let only = had_reference && info.AccumulatedFrames == 0;
            regions = pointer::with_moved(self.pointer.as_ref(), regions, only, &self.budget);
        }
        let picture = match resource.as_ref() {
            Some(resource) if changed => {
                if !release() {
                    regions = None;
                }
                Some(self.read_waiting(resource))
            }
            _ => None,
        };
        self.settle_frame(&duplication, resource, picture, regions, keyframe)
    }
    fn reopen(&mut self) -> io::Result<()> {
        self.reopen_settling()
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
