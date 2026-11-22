//! The ring of two NV12 pictures the capture thread writes and the encoder
//! reads (plan 2026-11-14 bildkette B-17, package BK2a; split from
//! `d3d_video_convert.rs` by plan 2026-11-17 P9). Two slots of the capture's
//! native size (even), so that a picture stays valid when the encoder's size
//! changes; one slot can be in the encoder (`busy`), the capture writes the
//! other. Each slot owns a claim of the capture grant
//! (`CaptureOwner::Readback`, the budget of the CPU staging texture it
//! replaces) taken before its texture is created. P9: the pointer of a
//! Desktop Duplication picture is drawn into the slot as a small patch
//! (`Ring::overlay`), so the picture keeps the GPU route.
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, ID3D11VideoProcessorOutputView,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;

use super::{nv12_len, nv12_texture, slot_size, VideoConverter};
use crate::screen::core::capture_admission::{CaptureGeneration, CaptureSlice, SourceKey};
use crate::screen::core::pixel_convert::{FrameSize, Rect};

/// One NV12 picture of the ring.
struct Slot {
    texture: ID3D11Texture2D,
    view: ID3D11VideoProcessorOutputView,
    /// Tag of the picture it holds (0: none yet).
    tag: u16,
    /// Handed to the encoder; the capture writes the other slot.
    busy: bool,
    // The texture is released before its claim.
    _claim: CaptureSlice,
}

struct RingGpu {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    convert: VideoConverter,
    slots: Vec<Slot>,
    next: usize,
}

// SAFETY: as `VideoConverter`; every use goes through the ring's mutex.
unsafe impl Send for RingGpu {}

/// The NV12 pictures of one capture generation on one device.
pub(in crate::screen::os) struct Ring {
    pub source: SourceKey,
    pub generation: CaptureGeneration,
    /// `ID3D11Device` pointer of the capture (identity only).
    pub device: usize,
    pub native: FrameSize,
    pub size: FrameSize,
    gpu: Mutex<RingGpu>,
    /// Codex round 9, 2: a slot was abandoned to a driver that may still
    /// read it (`GpuInput::abandon`); the capture takes a new ring.
    spoiled: AtomicBool,
}

impl Ring {
    /// Two slots, each claimed from `claim` before its texture exists.
    pub(in crate::screen::os) fn create(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        identity: (SourceKey, CaptureGeneration),
        native: FrameSize,
        mut claim: impl FnMut(u64) -> io::Result<CaptureSlice>,
    ) -> io::Result<Self> {
        let size = slot_size(native);
        let convert =
            VideoConverter::new(device, context, native, DXGI_FORMAT_B8G8R8A8_UNORM, size)?;
        let mut slots = Vec::with_capacity(2);
        for _ in 0..2 {
            let claim = claim(nv12_len(size))?;
            let texture = nv12_texture(device, size, None)?;
            let view = convert.output_view(&texture)?;
            slots.push(Slot {
                texture,
                view,
                tag: 0,
                busy: false,
                _claim: claim,
            });
        }
        Ok(Self {
            source: identity.0,
            generation: identity.1,
            device: device.as_raw() as usize,
            native,
            size,
            gpu: Mutex::new(RingGpu {
                device: device.clone(),
                context: context.clone(),
                convert,
                slots,
                next: 0,
            }),
            spoiled: AtomicBool::new(false),
        })
    }

    fn lock(&self) -> MutexGuard<'_, RingGpu> {
        self.gpu.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Converts the capture texture into the slot that is not in the
    /// encoder and gives it `tag`; with `patch` (P9) the pointer patch is
    /// drawn over its area. The work is submitted before the capture
    /// releases its frame (`Flush`). The outer error is the conversion's
    /// (the GPU path fails), the inner one the patch's (only the pointer
    /// cannot be drawn on the GPU); after either the slot holds no picture.
    pub(in crate::screen::os) fn write(
        &self,
        source: &ID3D11Texture2D,
        tag: u16,
        patch: Option<(&ID3D11Texture2D, Rect)>,
    ) -> io::Result<io::Result<()>> {
        let mut gpu = self.lock();
        let index = if gpu.slots[gpu.next].busy {
            gpu.next ^ 1
        } else {
            gpu.next
        };
        // Codex round 9, 2: never into a slot an encoder still reads; the
        // picture is refused for memory (`WouldBlock`) and read again.
        if gpu.slots[index].busy {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "both ring slots are with an encoder",
            ));
        }
        // The slot loses its old picture before the conversion can fail.
        gpu.slots[index].tag = 0;
        gpu.convert.convert(source, &gpu.slots[index].view)?;
        if let Some((texture, at)) = patch {
            if let Err(error) = gpu.convert.overlay(texture, at, &gpu.slots[index].view) {
                return Ok(Err(error));
            }
        }
        gpu.slots[index].tag = tag;
        gpu.next = index ^ 1;
        // SAFETY: submits the queued conversion of the live context.
        unsafe { gpu.context.Flush() };
        Ok(Ok(()))
    }

    /// Device and immediate context of the ring (readback of a picture).
    pub(in crate::screen::os) fn handles(&self) -> (ID3D11Device, ID3D11DeviceContext) {
        let gpu = self.lock();
        (gpu.device.clone(), gpu.context.clone())
    }

    /// The tags of both slots.
    pub(in crate::screen::os) fn tags(&self) -> [u16; 2] {
        let gpu = self.lock();
        [gpu.slots[0].tag, gpu.slots[1].tag]
    }

    /// The texture of slot `index` with `tag`, marked busy until `release`;
    /// `None` when the slot holds another picture by now.
    pub(in crate::screen::os) fn take(&self, index: usize, tag: u16) -> Option<ID3D11Texture2D> {
        let mut gpu = self.lock();
        let slot = gpu.slots.get_mut(index).filter(|slot| slot.tag == tag)?;
        slot.busy = true;
        Some(slot.texture.clone())
    }

    /// See `spoiled`; the abandoned slot stays busy for the ring's life.
    pub(in crate::screen::os) fn spoil(&self) {
        self.spoiled.store(true, Ordering::Release);
    }

    pub(in crate::screen::os) fn spoiled(&self) -> bool {
        self.spoiled.load(Ordering::Acquire)
    }

    pub(in crate::screen::os) fn release(&self, index: usize) {
        if let Some(slot) = self.lock().slots.get_mut(index) {
            slot.busy = false;
        }
    }

    /// Runs `body` with the texture of slot `index` while it holds `tag`;
    /// the capture cannot overwrite it meanwhile.
    pub(in crate::screen::os) fn with_slot<T>(
        &self,
        index: usize,
        tag: u16,
        body: impl FnOnce(&ID3D11Texture2D) -> io::Result<T>,
    ) -> io::Result<Option<T>> {
        let gpu = self.lock();
        match gpu.slots.get(index).filter(|slot| slot.tag == tag) {
            Some(slot) => body(&slot.texture).map(Some),
            None => Ok(None),
        }
    }
}
