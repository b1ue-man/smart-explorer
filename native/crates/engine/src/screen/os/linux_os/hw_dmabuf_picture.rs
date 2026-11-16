//! A DMA-BUF picture of the PipeWire capture as the GPU picture of the host
//! pipeline (plan 2026-11-14 „Bildkette“ BK8b hand-off Ü1, BK3b's
//! `capture_picture::GpuSurface`; joined by merge W2). The picture holds the
//! dequeued PipeWire buffer for the encoder (`take_held`, once) and the
//! descriptor of the picture for a read-back. The VA-API encoder finds it by
//! downcasting (`hw_encode_dmabuf.rs`); every other consumer gets CPU pixels
//! only through `GpuSurface::read_back`, which also asks the stream to leave
//! DMA-BUF for the session (`DmaBufCause::Consumer`, B-15: a stream whose
//! pictures are read back every time is better on shared memory).
//!
//! The read-back is exact: the buffer is imported as an RGB32 surface of
//! its own layout (`va_fourcc_for`: `XR24` → `VA_FOURCC_BGRX`, `AR24` →
//! `VA_FOURCC_BGRA`) and copied with `vaGetImage` into an image of the same
//! fourcc (`vaapi_surface::read_rgb32`), so no colour conversion and no NV12
//! is involved (libva 2.20.0 `va.h`, `vaGetImage`: "Retrive surface data
//! into a VAImage"; a conversion happens only between different formats);
//! the alpha byte is set to 255 as on the shared memory route
//! (`pipewire_frame_copy`). It is the still image's route too
//! (`LatestFrame::still_from_gpu`).
use std::any::Any;
use std::io;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use super::hw_encode_common::HeldDmaBuf;
use super::vaapi_dmabuf::read_back;
use super::vaapi_vpp::DmaBufFrame;
use crate::screen::core::capture_admission::{CaptureGrant, CaptureOwner};
use crate::screen::core::capture_image::{CaptureImageBuilder, CapturedImage};
use crate::screen::core::capture_picture::GpuSurface;
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::pixel_convert::FrameSize;

/// Why the GPU route of a stream ends; the stream maps it onto its own
/// fallback causes (`pipewire_dmabuf::Fallback`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::screen::os) enum DmaBufCause {
    /// `vaCreateSurfaces` refused the buffer.
    Import,
    /// The video processor failed on the imported buffer.
    Conversion,
    /// The session's encoder takes CPU pictures.
    Consumer,
}

/// The stream a DMA-BUF picture came from (implemented by the PipeWire
/// stream): leaves DMA-BUF for the rest of the session, renegotiating
/// shared memory without a new portal dialog.
pub(in crate::screen::os) trait DmaBufStream: Send + Sync {
    fn fall_back(&self, cause: DmaBufCause);
}

/// The mark of a failed conversion after a working import
/// (`vaapi_encode_base::VaEncodeBase::import_dmabuf`).
#[derive(Debug)]
pub(in crate::screen::os) struct ConversionFailed(pub String);

impl std::fmt::Display for ConversionFailed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "DMA-BUF conversion: {}", self.0)
    }
}

impl std::error::Error for ConversionFailed {}

/// The cause of a failed import step: a marked conversion, else the import.
pub(in crate::screen::os) fn cause_of(error: &io::Error) -> DmaBufCause {
    match error.get_ref() {
        Some(inner) if inner.is::<ConversionFailed>() => DmaBufCause::Conversion,
        _ => DmaBufCause::Import,
    }
}

/// Reads the DMA-BUF picture `frame` on `node` back once, exactly (BGRx of
/// the source, see the module text), into admitted capture pixels of
/// `owner`.
pub(in crate::screen::os) fn read_bgrx(
    node: &std::path::Path,
    frame: &DmaBufFrame,
    grant: &CaptureGrant,
    owner: CaptureOwner,
) -> io::Result<CapturedImage> {
    let mut image = CaptureImageBuilder::new(frame.size, FrameLimits::RESOURCE, grant, owner)?;
    read_back(node, frame, &mut |y, row| {
        let target = image
            .row_mut(y)
            .ok_or_else(|| io::Error::other("DMA-BUF row outside picture"))?;
        let width = target.len();
        target.copy_from_slice(
            row.get(..width)
                .ok_or_else(|| io::Error::other("DMA-BUF row shorter than picture"))?,
        );
        for pixel in target.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
        Ok(())
    })?;
    Ok(image.freeze())
}

/// One DMA-BUF picture of the capture on its way to the encoder.
pub(in crate::screen::os) struct DmaBufPicture {
    /// The dequeued buffer; `None` for a repeat or once the encoder took it.
    held: Mutex<Option<Box<dyn HeldDmaBuf + Send>>>,
    frame: DmaBufFrame,
    /// Keeps `frame.fd` open for a read-back after the buffer went back.
    _descriptor: Arc<OwnedFd>,
    node: PathBuf,
    stream: Arc<dyn DmaBufStream>,
    slot: u64,
}

impl DmaBufPicture {
    /// `frame` lives on `descriptor`; `held` is `None` for a key frame
    /// without a new picture (the encoder's input still holds it).
    pub(in crate::screen::os) fn new(
        held: Option<Box<dyn HeldDmaBuf + Send>>,
        (frame, descriptor): (DmaBufFrame, Arc<OwnedFd>),
        node: PathBuf,
        stream: Arc<dyn DmaBufStream>,
        slot: u64,
    ) -> Self {
        Self {
            held: Mutex::new(held),
            frame,
            _descriptor: descriptor,
            node,
            stream,
            slot,
        }
    }

    /// The dequeued buffer for the encoder, once; `None` re-encodes the
    /// picture in the encoder's input.
    pub(in crate::screen::os) fn take_held(&self) -> Option<Box<dyn HeldDmaBuf + Send>> {
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// Ends DMA-BUF for the stream's session (no lock of this picture is
    /// held while the stream renegotiates).
    pub(in crate::screen::os) fn fall_back(&self, cause: DmaBufCause) {
        self.stream.fall_back(cause);
    }

    /// The picture as CPU pixels of `size` without leaving DMA-BUF: the one
    /// read-back of a key frame for an encoder whose input is still empty.
    pub(in crate::screen::os) fn read_once(
        &self,
        size: FrameSize,
        grant: &CaptureGrant,
    ) -> io::Result<CapturedImage> {
        let image = read_bgrx(&self.node, &self.frame, grant, CaptureOwner::SourcePixels)?;
        if image.size() == size {
            return Ok(image);
        }
        image.downscale(
            size,
            FrameLimits::RESOURCE,
            grant,
            CaptureOwner::ScaledPixels,
        )
    }

    /// Whether the encoder's buffer is still held (task tests).
    #[cfg(test)]
    pub(in crate::screen::os) fn holds_buffer(&self) -> bool {
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }
}

impl GpuSurface for DmaBufPicture {
    fn native(&self) -> FrameSize {
        self.frame.size
    }
    fn slot(&self) -> u64 {
        self.slot
    }
    /// The descriptor keeps the buffer's memory; its content is the
    /// compositor's until a newer picture replaces it (BK8b A11).
    fn held(&self) -> bool {
        true
    }
    /// A CPU consumer: the pixels once, and shared memory for the session.
    fn read_back(
        &self,
        size: FrameSize,
        limits: FrameLimits,
        grant: &CaptureGrant,
        owner: CaptureOwner,
    ) -> io::Result<CapturedImage> {
        limits.check(size)?;
        // Read while the buffer is still held (its content is stable), then
        // give it back and leave DMA-BUF.
        let read = if size == self.frame.size {
            read_bgrx(&self.node, &self.frame, grant, owner)
        } else {
            read_bgrx(&self.node, &self.frame, grant, CaptureOwner::SourcePixels)
                .and_then(|image| image.downscale(size, limits, grant, CaptureOwner::ScaledPixels))
        };
        drop(self.take_held());
        self.fall_back(DmaBufCause::Consumer);
        read
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
