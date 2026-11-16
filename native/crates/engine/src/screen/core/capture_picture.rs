//! A captured picture in system memory or on the GPU (plan 2026-11-14
//! „Bildkette“, BK3b). A picture that a GPU encoder takes stays on the GPU of
//! its capture: no placeholder in system memory, no CPU scaling. Its type
//! keeps it apart from CPU pixels: every consumer that needs pixels in
//! memory (software encoders, tiles, still images, pointer blending, the
//! comparison „unchanged“) takes a `CapturedImage`, and a GPU picture becomes
//! one only through its readback (`GpuPicture::read_back`), which writes the
//! pixels into memory admitted like the CPU route's picture
//! (`CaptureOwner::SourcePixels` of the same grant).
//!
//! A platform supplies the surface (`GpuSurface`): on Windows a slot of the
//! NV12 ring of the capture device (`os/windows/mf_capture_media.rs`
//! `RingPicture`); on Linux (package BK8b, joined by merge W2) the held
//! PipeWire DMA-BUF (`os/linux_os/hw_dmabuf_picture.rs` `DmaBufPicture`),
//! which the VA-API encoder takes by downcasting `as_any` and imports into
//! its video processor (`hw_encode_dmabuf.rs`), as the Media Foundation
//! encoder does with the ring.
use std::any::Any;
use std::io;
use std::sync::Arc;

use super::capture_admission::{CaptureGeneration, CaptureGrant, CaptureOwner, SourceKey};
use super::capture_image::CapturedImage;
use super::frame_budget::FrameLimits;
use super::host_backend::SourceFrame;
use super::pixel_convert::FrameSize;
use super::session_state::SessionState;
use super::wire::invalid;

/// A picture held by the GPU of its capture.
pub trait GpuSurface: Send + Sync {
    /// Size of the surface; the picture may be delivered smaller.
    fn native(&self) -> FrameSize;
    /// The slot of the platform's ring that holds the picture (Windows: the
    /// ring tag), for diagnosis.
    fn slot(&self) -> u64;
    /// Whether the GPU still holds the picture (its ring was not retired).
    fn held(&self) -> bool;
    /// The picture as CPU pixels of `size` (scaled where it differs),
    /// admitted from `grant` as `owner`; `WouldBlock` when the GPU no longer
    /// holds it (the frame is skipped and the next one asked for).
    fn read_back(
        &self,
        size: FrameSize,
        limits: FrameLimits,
        grant: &CaptureGrant,
        owner: CaptureOwner,
    ) -> io::Result<CapturedImage>;
    /// For the encoder of the same platform, which takes its own surfaces.
    fn as_any(&self) -> &dyn Any;
}

/// A picture on the GPU: the surface, the size it is delivered at, and its
/// admission (the grant of the capture whose plan admitted the surface).
#[derive(Clone)]
pub struct GpuPicture {
    surface: Arc<dyn GpuSurface>,
    size: FrameSize,
    limits: FrameLimits,
    grant: CaptureGrant,
}

impl std::fmt::Debug for GpuPicture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GpuPicture")
            .field("size", &self.size)
            .field("slot", &self.surface.slot())
            .finish()
    }
}

impl GpuPicture {
    pub fn new(
        surface: Arc<dyn GpuSurface>,
        size: FrameSize,
        limits: FrameLimits,
        grant: &CaptureGrant,
    ) -> io::Result<Self> {
        if !grant.allows_images() {
            return Err(invalid("import credit cannot create a GPU picture"));
        }
        limits.check(size)?;
        let native = surface.native();
        if size.width > native.width.saturating_add(1)
            || size.height > native.height.saturating_add(1)
        {
            return Err(invalid("GPU picture is larger than its surface"));
        }
        Ok(Self {
            surface,
            size,
            limits,
            grant: grant.clone(),
        })
    }
    pub fn size(&self) -> FrameSize {
        self.size
    }
    pub fn source(&self) -> &SourceKey {
        self.grant.source()
    }
    pub fn generation(&self) -> CaptureGeneration {
        self.grant.generation()
    }
    pub fn grant(&self) -> CaptureGrant {
        self.grant.clone()
    }
    pub fn surface(&self) -> &Arc<dyn GpuSurface> {
        &self.surface
    }
    /// The same picture delivered at `size`: no pixel work, the consumer's
    /// GPU scales (the video processor of the encoder, or the readback).
    pub fn scaled(&self, size: FrameSize, limits: FrameLimits) -> io::Result<Self> {
        limits.check(size)?;
        if size.width > self.size.width || size.height > self.size.height {
            return Err(invalid("downscale target is larger than the source"));
        }
        Ok(Self {
            size,
            limits,
            ..self.clone()
        })
    }
    /// The CPU pixels of this picture, admitted like the CPU route's picture.
    pub fn read_back(&self) -> io::Result<CapturedImage> {
        READBACKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.surface.read_back(
            self.size,
            self.limits,
            &self.grant,
            CaptureOwner::SourcePixels,
        )
    }
}

/// GPU pictures this process read back to the CPU (plan 2026-11-14
/// „Bildkette“ BK0: the statistics count them, `readbacks`).
static READBACKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// GPU pictures this process read back so far.
pub fn readbacks() -> u64 {
    READBACKS.load(std::sync::atomic::Ordering::Relaxed)
}

/// A captured picture: CPU pixels, or a picture on the GPU.
#[derive(Clone, Debug)]
pub enum Picture {
    Cpu(CapturedImage),
    Gpu(GpuPicture),
}

impl Picture {
    pub fn size(&self) -> FrameSize {
        match self {
            Self::Cpu(image) => image.size(),
            Self::Gpu(picture) => picture.size(),
        }
    }
    pub fn source(&self) -> &SourceKey {
        match self {
            Self::Cpu(image) => image.source(),
            Self::Gpu(picture) => picture.source(),
        }
    }
    pub fn generation(&self) -> CaptureGeneration {
        match self {
            Self::Cpu(image) => image.generation(),
            Self::Gpu(picture) => picture.generation(),
        }
    }
    pub fn grant(&self) -> CaptureGrant {
        match self {
            Self::Cpu(image) => image.grant(),
            Self::Gpu(picture) => picture.grant(),
        }
    }
    /// CPU pixels: the CPU picture itself (shared, no copy), a GPU picture
    /// read back. The only way from a GPU picture to pixels in memory.
    pub fn to_cpu(&self) -> io::Result<CapturedImage> {
        match self {
            Self::Cpu(image) => Ok(image.clone()),
            Self::Gpu(picture) => picture.read_back(),
        }
    }
    /// The picture at `size`: a CPU picture scaled on the CPU as before, a
    /// GPU picture only relabelled (`GpuPicture::scaled`).
    pub fn scaled(
        &self,
        size: FrameSize,
        limits: FrameLimits,
        owner: CaptureOwner,
    ) -> io::Result<Self> {
        match self {
            Self::Cpu(image) if image.size() == size => Ok(self.clone()),
            Self::Cpu(image) => image
                .downscale(size, limits, &image.grant(), owner)
                .map(Self::Cpu),
            Self::Gpu(picture) => picture.scaled(size, limits).map(Self::Gpu),
        }
    }
    pub fn on_gpu(&self) -> bool {
        matches!(self, Self::Gpu(_))
    }
    /// Whether the pixels are still there: a CPU picture always, a GPU
    /// picture while its surface holds it (merge W1: a key frame of a
    /// picture no longer held needs a new capture, `capture_thread.rs`).
    pub fn held(&self) -> bool {
        match self {
            Self::Cpu(_) => true,
            Self::Gpu(picture) => picture.surface().held(),
        }
    }
}

/// What a capture delivered, a picture on the GPU included
/// (`ScreenSource::capture_picture`).
#[derive(Debug)]
pub enum SourcePicture {
    Frame(Picture),
    Unchanged,
    Paused(SessionState),
}

impl From<SourceFrame> for SourcePicture {
    fn from(frame: SourceFrame) -> Self {
        match frame {
            SourceFrame::Frame(image) => Self::Frame(Picture::Cpu(image)),
            SourceFrame::Unchanged => Self::Unchanged,
            SourceFrame::Paused(state) => Self::Paused(state),
        }
    }
}

impl SourcePicture {
    /// The frame with CPU pixels: a GPU picture is read back first.
    pub fn into_source_frame(self) -> io::Result<SourceFrame> {
        Ok(match self {
            Self::Frame(picture) => SourceFrame::Frame(picture.to_cpu()?),
            Self::Unchanged => SourceFrame::Unchanged,
            Self::Paused(state) => SourceFrame::Paused(state),
        })
    }
}

/// Encodes `picture` with an encoder that takes CPU pixels: a GPU picture
/// reaches it only read back (`ScreenEncoder::encode_picture`).
pub fn encode_on_cpu(
    encoder: &mut (impl super::host_backend::ScreenEncoder + ?Sized),
    picture: &Picture,
    request: &super::host_stage::StageRequest,
) -> io::Result<super::host_backend::CapturedEncodedFrame> {
    encoder.encode_captured(&picture.to_cpu()?, request)
}

#[cfg(test)]
#[path = "bildkette_gpu_picture_task_tests.rs"]
mod gpu_picture_tests;
