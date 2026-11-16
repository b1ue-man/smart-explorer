//! Admitted NV12 inputs. No temporary unclaimed NV12 Vec is materialized.
//!
//! Plan 2026-11-14 bildkette:
//! - B-19 (package BK2a): BGRA → NV12 on all cores, written straight into
//!   the MF buffer or a mapped staging texture. The conversion is the shared
//!   `pixel_convert::bgra_to_nv12_parallel` (BK3b: one function for Media
//!   Foundation and OpenH264, bit-identical to `bgra_to_i420_with_limits`,
//!   `YuvMatrix::Bt709`).
//! - BK3b: a picture of the GPU path is a `RingPicture`, the slot of the NV12
//!   ring that holds it, carried as `capture_picture::GpuPicture`; there is
//!   no placeholder in system memory any more. A texture encoder takes the
//!   slot (`mf_d3d_input.rs`), a system-memory encoder gets its NV12 copied
//!   from the slot (`stage`), every other consumer of CPU pixels its BGRA
//!   read back (`GpuSurface::read_back`).
use std::any::Any;
use std::io;
use std::sync::{Arc, Weak};

use super::mf_buffer_lease;
use super::mf_common::platform;
use super::mf_encoder::d3d_input::gone;
use super::mf_encoder::video_convert::{nv12_len, nv12_texture, with_mapped, Ring, VideoConverter};
use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner, CaptureSlice};
use crate::screen::core::capture_image::{CaptureImageBuilder, CapturedImage};
use crate::screen::core::capture_picture::{GpuPicture, GpuSurface, Picture};
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::pixel_convert::{
    bgra_to_nv12_parallel, nv12_to_bgra_resampled, BgraView, FrameSize, Nv12View, YuvMatrix,
};
use crate::screen::core::wire::invalid;
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ, D3D11_MAP, D3D11_MAP_READ,
    D3D11_MAP_WRITE,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_NV12;
use windows::Win32::Media::MediaFoundation::{IMFMediaBuffer, IMFSample};

/// BGRA → NV12 (BT.709 limited range) with luma rows `pitch` apart in `y`
/// and chroma rows `pitch` apart in `uv`, on all cores (B-19).
pub(in crate::screen::os) fn bgra_to_nv12(
    src: &BgraView,
    y: &mut [u8],
    uv: &mut [u8],
    pitch: usize,
) -> io::Result<()> {
    bgra_to_nv12_parallel(src, YuvMatrix::Bt709, y, uv, pitch)
}

/// Ring tags run 1..=511; 0 marks an empty slot.
pub(in crate::screen::os) const TAG_COUNT: u16 = 511;

pub(in crate::screen::os) fn next_tag(tag: u16) -> u16 {
    if tag >= TAG_COUNT {
        1
    } else {
        tag + 1
    }
}

/// `a` was given after `b` (less than half the cycle ago).
pub(in crate::screen::os) fn newer(a: u16, b: u16) -> bool {
    let distance = (a + TAG_COUNT - b) % TAG_COUNT;
    a != 0 && b != 0 && distance > 0 && distance < TAG_COUNT / 2
}

/// The slot that holds `tag`, else the newest slot holding a later picture
/// (the screen as it is now; the picture of `tag` was overwritten).
pub(in crate::screen::os) fn pick(tag: u16, slots: [u16; 2]) -> Option<usize> {
    if let Some(index) = slots.iter().position(|slot| *slot != 0 && *slot == tag) {
        return Some(index);
    }
    let later: Vec<usize> = (0..2).filter(|i| newer(slots[*i], tag)).collect();
    match later.as_slice() {
        [one] => Some(*one),
        [a, b] => Some(if newer(slots[*a], slots[*b]) { *a } else { *b }),
        _ => None,
    }
}

/// A picture in a slot of the NV12 ring of its capture (BK3b). The ring is
/// held weakly: a retired ring takes its pictures with it, and a consumer
/// then skips the frame and asks for the next one (`gone`), as before.
pub(in crate::screen::os) struct RingPicture {
    pub ring: Weak<Ring>,
    pub tag: u16,
    /// The ring's slot size (the capture's native size, even).
    pub size: FrameSize,
}

impl RingPicture {
    /// The ring and the slot that holds the picture, or a newer one.
    pub(in crate::screen::os) fn find(&self) -> io::Result<(Arc<Ring>, usize, u16)> {
        let ring = self.ring.upgrade().ok_or_else(gone)?;
        let tags = ring.tags();
        let index = pick(self.tag, tags).ok_or_else(gone)?;
        Ok((ring, index, tags[index]))
    }
}

/// The ring slot behind a GPU picture of this platform.
pub(in crate::screen::os) fn ring_picture(picture: &GpuPicture) -> io::Result<&RingPicture> {
    picture
        .surface()
        .as_any()
        .downcast_ref::<RingPicture>()
        .ok_or_else(|| invalid("GPU picture of another platform"))
}

impl GpuSurface for RingPicture {
    fn native(&self) -> FrameSize {
        self.size
    }
    fn slot(&self) -> u64 {
        u64::from(self.tag)
    }
    fn held(&self) -> bool {
        self.find().is_ok()
    }
    /// NV12 of the slot copied on the GPU into a staging texture (at the
    /// picture's size where NV12 can have it, scaled by the video
    /// processor), mapped and converted into BGRA admitted as `owner`.
    fn read_back(
        &self,
        size: FrameSize,
        limits: FrameLimits,
        grant: &CaptureGrant,
        owner: CaptureOwner,
    ) -> io::Result<CapturedImage> {
        let (ring, index, tag) = self.find()?;
        let fits = size.width <= ring.size.width && size.height <= ring.size.height;
        let staged_size = if size == size.even() && fits {
            size
        } else {
            ring.size
        };
        let staged = stage(&ring, index, tag, grant, staged_size)?;
        drop(ring);
        let mut out = CaptureImageBuilder::new(size, limits, grant, owner)?;
        staged.map(D3D11_MAP_READ, |y, uv, pitch| {
            let limits = FrameLimits::RESOURCE;
            let view = Nv12View::new_with_limits(staged_size, y, pitch, uv, pitch, limits)?;
            nv12_to_bgra_resampled(&view, YuvMatrix::Bt709, out.pixels_mut(), size)
        })?;
        Ok(out.freeze())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// CPU picture into the encoder's texture: converted on all cores into the
/// mapped staging texture, then copied on the GPU.
pub(in crate::screen::os) fn upload(
    context: &ID3D11DeviceContext,
    view: &BgraView,
    staging: &ID3D11Texture2D,
    target: &ID3D11Texture2D,
) -> io::Result<()> {
    with_mapped(
        context,
        staging,
        view.size(),
        D3D11_MAP_WRITE,
        |y, uv, pitch| bgra_to_nv12(view, y, uv, pitch),
    )?;
    // SAFETY: two live NV12 textures of the same size and device.
    unsafe { context.CopyResource(target, staging) };
    Ok(())
}

/// NV12 of a GPU picture in a staging texture the CPU can read.
pub(in crate::screen::os) struct StagedPicture {
    context: ID3D11DeviceContext,
    staging: ID3D11Texture2D,
    size: FrameSize,
    _claim: CaptureSlice,
}

impl StagedPicture {
    /// Maps the staging texture for `body`; a staging texture fails to map
    /// only with its device: the frame is skipped.
    fn map<T>(
        &self,
        mode: D3D11_MAP,
        body: impl FnOnce(&mut [u8], &mut [u8], usize) -> io::Result<T>,
    ) -> io::Result<T> {
        let mut mapped = false;
        let result = with_mapped(&self.context, &self.staging, self.size, mode, |y, uv, p| {
            mapped = true;
            body(y, uv, p)
        });
        match result {
            Err(_) if !mapped => Err(gone()),
            result => result,
        }
    }

    /// Copies the planes into a tight NV12 buffer of the picture's size.
    fn copy_into(&self, target: &mut [u8]) -> io::Result<()> {
        let (width, height) = (self.size.width as usize, self.size.height as usize);
        if target.len() as u64 != nv12_len(self.size) {
            return Err(invalid("NV12 target length differs"));
        }
        self.map(D3D11_MAP_READ, |y, uv, pitch| {
            let (luma, chroma) = target.split_at_mut(width * height);
            for (row, out) in luma.chunks_exact_mut(width).enumerate() {
                out.copy_from_slice(&y[row * pitch..row * pitch + width]);
            }
            for (row, out) in chroma.chunks_exact_mut(width).enumerate() {
                out.copy_from_slice(&uv[row * pitch..row * pitch + width]);
            }
            Ok(())
        })
    }
}

/// Claims the staging slice (`CodecInput`) and, where `ring_size` differs
/// from `size`, the temporary render-target slice (`ScaledPixels`, the
/// session's scaling budget, which a GPU picture has not claimed yet).
/// Dropping the scaling slice before returning keeps at most two
/// `CodecInput` claims across staging and the MF buffer (`input_sample`).
pub(in crate::screen::os) fn stage_claims<T>(
    grant: &CaptureGrant,
    ring_size: FrameSize,
    size: FrameSize,
    same: impl FnOnce(&CaptureSlice) -> io::Result<T>,
    scaled: impl FnOnce(&CaptureSlice, &CaptureSlice) -> io::Result<T>,
) -> io::Result<(T, CaptureSlice)> {
    let bytes = nv12_len(size);
    let staging = grant.claim(CaptureOwner::CodecInput, bytes)?;
    let out = if ring_size == size {
        same(&staging)?
    } else {
        let temp = grant.claim(CaptureOwner::ScaledPixels, bytes)?;
        scaled(&staging, &temp)?
    };
    Ok((out, staging))
}

/// The NV12 of ring slot `index` (holding `tag`) at `size`, in a staging
/// texture claimed from `grant` (`CodecInput`, the budget of the CPU
/// conversion it replaces); scaled by the video processor where `size`
/// differs from the slot (`ScaledPixels`). The skip error when the slot moved on.
fn stage(
    ring: &Ring,
    index: usize,
    tag: u16,
    grant: &CaptureGrant,
    size: FrameSize,
) -> io::Result<StagedPicture> {
    let (device, context) = ring.handles();
    // SAFETY: a plain query of the live device. A lost device takes its
    // pictures with it: the frame is skipped until the capture rebuilds.
    if unsafe { device.GetDeviceRemovedReason() }.is_err() {
        return Err(gone());
    }
    let (staging, claim) = stage_claims(
        grant,
        ring.size,
        size,
        |_| {
            let staging = nv12_texture(&device, size, Some(D3D11_CPU_ACCESS_READ))?;
            ring.with_slot(index, tag, |source: &ID3D11Texture2D| {
                // SAFETY: two live NV12 textures of the same size and device.
                unsafe { context.CopyResource(&staging, source) };
                Ok(())
            })?
            .ok_or_else(gone)?;
            Ok(staging)
        },
        |_, _| {
            let staging = nv12_texture(&device, size, Some(D3D11_CPU_ACCESS_READ))?;
            let scaled = nv12_texture(&device, size, None)?;
            let converter =
                VideoConverter::new(&device, &context, ring.size, DXGI_FORMAT_NV12, size)?;
            let view = converter.output_view(&scaled)?;
            ring.with_slot(index, tag, |slot: &ID3D11Texture2D| {
                converter.convert(slot, &view)
            })?
            .ok_or_else(gone)?;
            // SAFETY: two live NV12 textures of the same size and device.
            unsafe { context.CopyResource(&staging, &scaled) };
            Ok(staging)
        },
    )?;
    Ok(StagedPicture {
        context,
        staging,
        size,
        _claim: claim,
    })
}

pub(super) fn sample_with_buffer(buffer: &IMFMediaBuffer) -> io::Result<IMFSample> {
    let mut raw = std::ptr::null_mut();
    unsafe { (platform()?.create_sample)(&mut raw) }.ok()?;
    let sample = unsafe { IMFSample::from_raw(raw) };
    // AddBuffer holds this wrapper's identity, not its private inner buffer.
    unsafe { sample.AddBuffer(buffer)? };
    Ok(sample)
}

/// The system-memory input sample of `picture`: a CPU picture converted
/// straight into the MF buffer, a GPU picture's NV12 copied from its slot.
pub(super) fn input_sample(
    grant: &CaptureGrant,
    picture: &Picture,
    time: i64,
    duration: i64,
) -> io::Result<IMFSample> {
    grant.check_identity(picture.source(), picture.generation())?;
    let size = picture.size();
    FrameLimits::RESOURCE.check(size)?;
    if size != size.even() {
        return Err(invalid("Media Foundation needs even NV12 geometry"));
    }
    let length = nv12_len(size);
    // A GPU picture is staged before the MF buffer's claim.
    let staged = match picture {
        Picture::Gpu(gpu) => {
            let (ring, index, tag) = ring_picture(gpu)?.find()?;
            Some(stage(&ring, index, tag, grant, size)?)
        }
        Picture::Cpu(_) => None,
    };
    let buffer = mf_buffer_lease::create(
        grant,
        CaptureOwner::CodecInput,
        usize::try_from(length).map_err(|_| invalid("MF input exceeds address space"))?,
    )?;
    let mut data = std::ptr::null_mut();
    let mut capacity = 0u32;
    unsafe { buffer.Lock(&mut data, Some(&mut capacity), None)? };
    let filled = (|| -> io::Result<()> {
        if data.is_null() || u64::from(capacity) < length {
            return Err(invalid("MF input buffer contradicts its allocation size"));
        }
        // SAFETY: the locked native buffer has at least length writable bytes.
        let target = unsafe { std::slice::from_raw_parts_mut(data, length as usize) };
        match (&staged, picture) {
            (Some(staged), _) => staged.copy_into(target),
            (None, Picture::Cpu(image)) => {
                let width = size.width as usize;
                let (y, uv) = target.split_at_mut(width * size.height as usize);
                bgra_to_nv12(&image.view(), y, uv, width)
            }
            (None, Picture::Gpu(_)) => Err(gone()),
        }
    })();
    // Always unlock even when size verification failed, then release the
    // staging slice so only the MF buffer stays claimed during ProcessInput.
    let unlocked = unsafe { buffer.Unlock() };
    drop(staged);
    filled?;
    unlocked?;
    unsafe { buffer.SetCurrentLength(length as u32)? };
    let sample = sample_with_buffer(&buffer)?;
    unsafe {
        sample.SetSampleTime(time)?;
        sample.SetSampleDuration(duration)?;
    }
    Ok(sample)
}
