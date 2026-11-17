//! Textures as input of the hardware encoder (plan 2026-11-14 bildkette
//! B-17/B-20, package BK2a, ranks of 4.10a): the picture stays on the GPU of
//! the capture.
//!
//! - The hardware MFT gets a DXGI device manager of the capture's device
//!   (`MFT_MESSAGE_SET_D3D_MANAGER`) and arms the capture (`attach`).
//! - While armed, `capture_readback` converts the capture texture with the
//!   video processor into the NV12 ring (`d3d_video_convert.rs`) before the
//!   frame is released, and the pipeline gets a GPU picture
//!   (`capture_picture::GpuPicture` around `mf_capture_media::RingPicture`,
//!   BK3b) that names the ring slot; no pixels go through system memory.
//! - The encoder takes the slot (`GpuInput::sample`), scales it on the GPU
//!   where its size differs, and wraps it with `MFCreateDXGISurfaceBuffer`;
//!   a picture of the CPU path is converted on all cores into a staging
//!   texture and copied up.
//! - After any GPU failure the CPU path holds for the rest of the session
//!   (`disable`, sticky in this host process); `capture_route` names the
//!   other cases of the CPU path.
//! - Package BK12 (B-22): the vendor encoders NVENC and AMF open on the
//!   capture's device itself (`attach_device`, no device manager) and take
//!   the same textures (`GpuInput::texture`).
//! - Plan 2026-11-17 P3 (b): without the capture's device (none yet,
//!   another adapter or vendor) they get a device of their own with the
//!   CPU upload (`mf_d3d_own.rs`).
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, ID3D11VideoProcessorOutputView,
    D3D11_CPU_ACCESS_FLAG,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_NV12;
use windows::Win32::Media::MediaFoundation::{
    IMFDXGIDeviceManager, IMFSample, IMFTransform, MF_SA_D3D11_AWARE,
};

#[path = "mf_d3d_own.rs"]
mod own;
#[path = "mf_d3d_surface.rs"]
mod surface;
pub(in crate::screen::os) use own::{attach_own, input_route, own_for_vendor, InputRoute};
pub(in crate::screen::os) use surface::detach;

use super::video_convert::{nv12_len, nv12_texture, Ring, VideoConverter};
use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner, CaptureSlice};
use crate::screen::core::capture_picture::Picture;
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::os::capture_readback::{capture_route, CaptureFacts, CpuReason};
use crate::screen::os::d3d_device::DeviceSet;
use crate::screen::os::mf_capture_media::{next_tag, ring_picture, upload, RingPicture};
use crate::screen::os::mf_common::adapter_of;

/// Why a hardware encoder takes NV12 in system memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::screen::os) enum SystemReason {
    Disabled,
    /// No capture device seen yet in this process.
    NoCapture,
    NoVideo,
    /// The MFT does not report `MF_SA_D3D11_AWARE`.
    NotD3D11Aware,
    /// MFT and capture on different adapters (hybrid laptops).
    OtherAdapter,
    ManagerRefused,
    /// BK12: the capture's device is not the vendor encoder's GPU.
    OtherVendor,
}

/// What the encoder side knows when a hardware MFT opens.
#[derive(Clone, Copy, Debug)]
pub(in crate::screen::os) struct EncoderFacts {
    pub disabled: bool,
    pub capture: bool,
    pub video: bool,
    pub aware: bool,
    pub capture_adapter: Option<u64>,
    pub encoder_adapter: Option<u64>,
}

/// Decision B-20 at open: textures only into an MFT that is D3D11-aware and
/// on the capture's adapter (an MFT without the LUID attribute is tried; its
/// refusal of the manager keeps system memory).
pub(in crate::screen::os) fn encoder_route(facts: EncoderFacts) -> Result<(), SystemReason> {
    let adapters = (facts.capture_adapter, facts.encoder_adapter);
    let other_adapter = matches!(adapters, (Some(capture), Some(encoder)) if capture != encoder);
    [
        (facts.disabled, SystemReason::Disabled),
        (!facts.capture, SystemReason::NoCapture),
        (!facts.video, SystemReason::NoVideo),
        (!facts.aware, SystemReason::NotD3D11Aware),
        (other_adapter, SystemReason::OtherAdapter),
    ]
    .into_iter()
    .find_map(|(refused, reason)| refused.then_some(reason))
    .map_or(Ok(()), Err)
}

/// The device the capture reads its pictures on.
struct Capture {
    raw: usize,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    luid: Option<u64>,
    video: bool,
}

/// Process-wide state between the capture thread and the encoder.
struct Bridge {
    capture: Option<Capture>,
    /// Device pointer and token of the encoder that takes textures.
    armed: Option<(usize, u64)>,
    disabled: Option<String>,
    ring: Option<Arc<Ring>>,
    tag: u16,
    token: u64,
    /// A still image asked for the exact BGRA of the next picture (merge
    /// W1): that picture is read back once on the CPU route, the ring stays.
    exact: bool,
}

static BRIDGE: Mutex<Bridge> = Mutex::new(Bridge {
    capture: None,
    armed: None,
    disabled: None,
    ring: None,
    tag: 0,
    token: 0,
    exact: false,
});

fn bridge() -> MutexGuard<'static, Bridge> {
    BRIDGE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A GPU picture that is no longer held: the frame is skipped and the
/// encoder asks for the next one.
pub(in crate::screen::os) fn gone() -> io::Error {
    io::Error::new(io::ErrorKind::WouldBlock, "GPU picture is no longer held")
}

/// The CPU path for the rest of this session (host process).
pub(in crate::screen::os) fn disable(reason: &str) {
    let mut state = bridge();
    if state.disabled.is_none() {
        eprintln!("screen GPU path off for this session, CPU path from now on: {reason}");
        state.disabled = Some(reason.to_string());
    }
    state.armed = None;
}

/// A still image (`se screen snapshot`) asks for the exact BGRA of the next
/// picture: `capture_readback` reads it once into memory admitted like the
/// CPU route's picture instead of converting it into the NV12 ring.
pub(in crate::screen::os) fn request_exact() {
    bridge().exact = true;
}

/// The adapter (packed LUID) the capture of this process reads its
/// pictures on (merge W1: the hardware MFT of this adapter opens first).
pub(in crate::screen::os) fn capture_adapter() -> Option<u64> {
    bridge().capture.as_ref().and_then(|capture| capture.luid)
}

/// Waits (bounded) until no encoder uses `ring`, then drops it with its
/// claims, so the CPU staging texture can take their budget.
fn retire(ring: Option<Arc<Ring>>) {
    let Some(ring) = ring else { return };
    let until = Instant::now() + Duration::from_millis(250);
    while Arc::strong_count(&ring) > 1 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Drops the ring (end of a capture generation, or the CPU path follows).
pub(in crate::screen::os) fn release_ring() {
    let ring = bridge().ring.take();
    retire(ring);
}

/// Capture side: notes the capture's device and decides the route of the
/// picture; on the CPU route the ring is released first.
pub(in crate::screen::os) fn route_capture(
    devices: &DeviceSet,
    facts: impl FnOnce(Option<usize>, bool, bool) -> CaptureFacts,
) -> Result<(), CpuReason> {
    let raw = devices.device.as_raw() as usize;
    let mut state = bridge();
    if state.capture.as_ref().map(|capture| capture.raw) != Some(raw) {
        state.capture = Some(Capture {
            raw,
            device: devices.device.clone(),
            context: devices.context.clone(),
            luid: devices.adapter_luid(),
            video: devices.has_video(),
        });
    }
    let video = state.capture.as_ref().is_some_and(|capture| capture.video);
    let decided = capture_route(facts(
        state.armed.map(|(device, _)| device),
        state.disabled.is_some(),
        video,
    ));
    // A still image takes this picture exactly (the CPU route is exact
    // anyway); the GPU route keeps its ring for the next pictures.
    if std::mem::take(&mut state.exact) && decided.is_ok() {
        return Err(CpuReason::Exact);
    }
    if decided.is_err() {
        let ring = state.ring.take();
        drop(state);
        retire(ring);
    }
    decided
}

/// The ring for this capture generation and the tag of the next picture;
/// a new ring replaces the old one after `drop_staging` freed the CPU
/// staging budget it takes. `create` builds and claims the new ring.
pub(in crate::screen::os) fn ring_for(
    matches: impl Fn(&Ring) -> bool,
    drop_staging: impl FnOnce(),
    create: impl FnOnce() -> io::Result<Ring>,
) -> io::Result<(Arc<Ring>, u16)> {
    let mut state = bridge();
    state.tag = next_tag(state.tag);
    let tag = state.tag;
    if let Some(ring) = state.ring.clone().filter(|ring| matches(ring)) {
        return Ok((ring, tag));
    }
    let old = state.ring.take();
    drop(state);
    retire(old);
    drop_staging();
    let ring = Arc::new(create()?);
    bridge().ring = Some(ring.clone());
    Ok((ring, tag))
}

/// An NV12 texture with its claim (`CodecInput`, the budget of the CPU
/// conversion it replaces).
pub(in crate::screen::os) struct Owned {
    pub texture: ID3D11Texture2D,
    pub claim: CaptureSlice,
}

pub(in crate::screen::os) fn owned(
    device: &ID3D11Device,
    grant: &CaptureGrant,
    size: FrameSize,
    cpu: Option<D3D11_CPU_ACCESS_FLAG>,
) -> io::Result<Owned> {
    let claim = grant.claim(CaptureOwner::CodecInput, nv12_len(size))?;
    Ok(Owned {
        texture: nv12_texture(device, size, cpu)?,
        claim,
    })
}

/// Encoder side: the device manager handed to the MFT (none for a vendor
/// encoder of BK12) and its inputs.
pub(in crate::screen::os) struct GpuInput {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    raw: usize,
    _manager: Option<IMFDXGIDeviceManager>,
    size: FrameSize,
    /// The arming of the capture; `None` for a device of its own (P3).
    token: Option<u64>,
    /// Input texture of the encoder's size (scaled or uploaded pictures).
    target: Option<Owned>,
    upload: Option<Owned>,
    /// Target and staging texture of pictures without admission (P3).
    plain: Option<(ID3D11Texture2D, ID3D11Texture2D)>,
    scale: Option<(VideoConverter, ID3D11VideoProcessorOutputView)>,
    /// Ring slot the MFT reads until its output arrived.
    in_flight: Option<(Arc<Ring>, usize)>,
}

/// Hands the capture's device manager to `transform` (before its media
/// types are set) and arms the capture; else why it stays on system memory.
/// `adapter` is the MFT's adapter from the enumeration (merge W1); without
/// it the transform's own `MFT_ENUM_ADAPTER_LUID` blob is read (a `LUID`
/// blob, not a `UINT64` as BK2a read it: Chromium `GetBlob`).
pub(in crate::screen::os) fn attach(
    transform: &IMFTransform,
    size: FrameSize,
    adapter: Option<u64>,
) -> Result<GpuInput, SystemReason> {
    // SAFETY: attribute reads on the activated transform.
    let attributes = unsafe { transform.GetAttributes() }.ok();
    let aware = attributes
        .as_ref()
        .and_then(|a| unsafe { a.GetUINT32(&MF_SA_D3D11_AWARE) }.ok());
    let adapter = adapter.or_else(|| attributes.as_ref().and_then(adapter_of));
    let aware = aware.is_some_and(|value| value != 0);
    arm(size, aware, adapter, |device| {
        surface::hand_manager(transform, device)
            .map(Some)
            .map_err(|error| {
                eprintln!("screen encoder refused the capture's D3D11 device: {error}");
                SystemReason::ManagerRefused
            })
    })
}

/// BK12: arms the capture for a vendor encoder (NVENC, AMF) that opens on
/// the capture's device itself; `accepts` tells whether that device is the
/// vendor's GPU. No message on refusal: the next encoder simply follows.
pub(in crate::screen::os) fn attach_device(
    size: FrameSize,
    accepts: impl FnOnce(&ID3D11Device) -> bool,
) -> Result<GpuInput, SystemReason> {
    arm(size, true, None, |device| {
        accepts(device)
            .then_some(None)
            .ok_or(SystemReason::OtherVendor)
    })
}

/// The route of B-20 with the capture this process saw; `bind` gives the
/// device to the encoder (an MFT's device manager, none for BK12).
fn arm(
    size: FrameSize,
    aware: bool,
    encoder_adapter: Option<u64>,
    bind: impl FnOnce(&ID3D11Device) -> Result<Option<IMFDXGIDeviceManager>, SystemReason>,
) -> Result<GpuInput, SystemReason> {
    let (capture, disabled) = {
        let state = bridge();
        let capture = state
            .capture
            .as_ref()
            .map(|c| (c.raw, c.device.clone(), c.context.clone(), c.luid, c.video));
        (capture, state.disabled.is_some())
    };
    encoder_route(EncoderFacts {
        disabled,
        capture: capture.is_some(),
        video: capture.as_ref().is_some_and(|c| c.4),
        aware,
        capture_adapter: capture.as_ref().and_then(|c| c.3),
        encoder_adapter,
    })?;
    let Some((raw, device, context, _, _)) = capture else {
        return Err(SystemReason::NoCapture);
    };
    let manager = bind(&device)?;
    let mut state = bridge();
    state.token = state.token.wrapping_add(1);
    let token = state.token;
    state.armed = Some((raw, token));
    Ok(GpuInput {
        device,
        context,
        raw,
        _manager: manager,
        size,
        token: Some(token),
        target: None,
        upload: None,
        plain: None,
        scale: None,
        in_flight: None,
    })
}

impl GpuInput {
    /// The encoder gave the output of the last input: its slot is free again.
    pub(in crate::screen::os) fn finish(&mut self) {
        if let Some((ring, index)) = self.in_flight.take() {
            ring.release(index);
        }
    }

    /// The capture's device the textures live on (BK12 opens NVENC and AMF
    /// on it).
    pub(in crate::screen::os) fn device(&self) -> &ID3D11Device {
        &self.device
    }

    /// The input sample of `picture` for the MFT (`texture` in a DXGI buffer).
    pub(in crate::screen::os) fn sample(
        &mut self,
        grant: &CaptureGrant,
        picture: &Picture,
        time: i64,
        duration: i64,
    ) -> io::Result<IMFSample> {
        let texture = self.texture(grant, picture)?;
        surface::surface_sample(&texture, time, duration)
    }

    /// The input texture of `picture`: the ring texture of a GPU picture
    /// (scaled to the encoder's size where needed), or the CPU picture
    /// converted on all cores and copied into a texture. The ring slot stays
    /// held until `finish`.
    pub(in crate::screen::os) fn texture(
        &mut self,
        grant: &CaptureGrant,
        picture: &Picture,
    ) -> io::Result<ID3D11Texture2D> {
        self.finish();
        grant.check_identity(picture.source(), picture.generation())?;
        if picture.size() != self.size {
            return Err(io::Error::other("picture size differs from the encoder"));
        }
        let texture = match picture {
            Picture::Gpu(gpu) => self.from_ring(grant, ring_picture(gpu)?)?,
            Picture::Cpu(image) => {
                let target = self.target(grant)?;
                if self.upload.is_none() {
                    let write = Some(windows::Win32::Graphics::Direct3D11::D3D11_CPU_ACCESS_WRITE);
                    self.upload = Some(owned(&self.device, grant, self.size, write)?);
                }
                let staging = self
                    .upload
                    .as_ref()
                    .map(|u| u.texture.clone())
                    .ok_or_else(gone)?;
                upload(&self.context, &image.view(), &staging, &target)?;
                target
            }
        };
        Ok(texture)
    }

    /// The encoder-size input texture, created on first use.
    fn target(&mut self, grant: &CaptureGrant) -> io::Result<ID3D11Texture2D> {
        if self.target.is_none() {
            self.target = Some(owned(&self.device, grant, self.size, None)?);
        }
        self.target
            .as_ref()
            .map(|t| t.texture.clone())
            .ok_or_else(gone)
    }

    fn from_ring(
        &mut self,
        grant: &CaptureGrant,
        picture: &RingPicture,
    ) -> io::Result<ID3D11Texture2D> {
        let (ring, index, tag) = picture.find()?;
        if ring.device != self.raw {
            return Err(gone());
        }
        if ring.size == self.size {
            let texture = ring.take(index, tag).ok_or_else(gone)?;
            self.in_flight = Some((ring, index));
            return Ok(texture);
        }
        let target = self.target(grant)?;
        if self
            .scale
            .as_ref()
            .is_none_or(|(c, _)| c.input != ring.size)
        {
            self.scale = None;
            let (from, to) = (ring.size, self.size);
            let converter =
                VideoConverter::new(&self.device, &self.context, from, DXGI_FORMAT_NV12, to)?;
            let view = converter.output_view(&target)?;
            self.scale = Some((converter, view));
        }
        let (converter, view) = self.scale.as_ref().ok_or_else(gone)?;
        ring.with_slot(index, tag, |slot| converter.convert(slot, view))?
            .ok_or_else(gone)?;
        Ok(target)
    }
}

impl Drop for GpuInput {
    fn drop(&mut self) {
        self.finish();
        let Some(own) = self.token else {
            return;
        };
        let mut state = bridge();
        if state.armed.map(|(_, token)| token) == Some(own) {
            state.armed = None;
        }
    }
}
