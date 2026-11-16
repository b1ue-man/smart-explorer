//! The capture of a portal source (SCR-M5; merge W2 joins BK8b's DMA-BUF
//! route to BK3b's picture types): `capture_picture` hands the newest
//! DMA-BUF picture on as a GPU picture while DMA-BUF is negotiated
//! (`LatestFrame::take_gpu_picture`), else the shared memory picture as
//! before; `capture` (CPU pixels) reads a GPU picture back, which ends
//! DMA-BUF for the session (a CPU consumer, `GpuSurface::read_back`); a
//! still image reads the newest picture back once without leaving DMA-BUF
//! (`LatestFrame::still_from_gpu`, exactly the BGRx of the source).
use std::io;
use std::sync::Arc;

use super::super::pipewire_stream::{GpuCapture, LatestFrame, Taken};
use super::{PortalSource, PortalState, PIPEWIRE_MISSING};
use crate::screen::core::capture_admission::CaptureOwner;
use crate::screen::core::capture_picture::{Picture, SourcePicture};
use crate::screen::core::frame_budget::FrameContract;
use crate::screen::core::host_backend::{ScreenSource, SourceFrame};
use crate::screen::core::session_state::SessionState;

/// What the portal session holds for a capture.
enum Stream {
    Running(Arc<LatestFrame>),
    Answer(SourceFrame),
}

impl PortalSource {
    /// The running stream, or the answer a capture gives without one.
    fn stream(&self) -> io::Result<Stream> {
        Ok(match &*self.handle.lock() {
            PortalState::Waiting | PortalState::WaitingInactive => {
                Stream::Answer(SourceFrame::Paused(SessionState::WaitingForPortal))
            }
            PortalState::Streaming(latest) => Stream::Running(latest.clone()),
            PortalState::Denied(text) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("portal_denied: {text}"),
                ))
            }
            PortalState::Failed(text) if text == PIPEWIRE_MISSING => {
                Stream::Answer(SourceFrame::Paused(SessionState::NoDisplay))
            }
            PortalState::Failed(text) => return Err(io::Error::other(text.clone())),
        })
    }

    /// `ScreenSource::capture_picture`: a GPU picture on the DMA-BUF route.
    pub(super) fn capture_gpu(&mut self, keyframe: bool) -> io::Result<SourcePicture> {
        #[cfg(any(test, feature = "test-support"))]
        self.memory().activate_for_test()?;
        let latest = match self.stream()? {
            Stream::Running(latest) => latest,
            Stream::Answer(answer) => return Ok(answer.into()),
        };
        let gpu = match self.capture_grant() {
            Some(grant) => latest.take_gpu_picture(keyframe, &grant)?,
            None => None,
        };
        match gpu {
            Some(GpuCapture::Picture(picture)) => {
                self.size = picture.size();
                self.had_frame = true;
                Ok(SourcePicture::Frame(Picture::Gpu(picture)))
            }
            Some(GpuCapture::Unchanged) if !self.had_frame => {
                Ok(SourcePicture::Paused(SessionState::WaitingForPortal))
            }
            Some(GpuCapture::Unchanged) => Ok(SourcePicture::Unchanged),
            None => self.take_shared(&latest, keyframe).map(SourcePicture::from),
        }
    }

    /// The shared memory picture of the stream (the route before BK8b).
    fn take_shared(&mut self, latest: &LatestFrame, keyframe: bool) -> io::Result<SourceFrame> {
        match latest.take(keyframe) {
            Taken::Frame(image) => {
                self.size = image.size();
                self.had_frame = true;
                Ok(SourceFrame::Frame(image))
            }
            Taken::Unchanged if !self.had_frame => {
                Ok(SourceFrame::Paused(SessionState::WaitingForPortal))
            }
            Taken::Unchanged => Ok(SourceFrame::Unchanged),
            Taken::Failed(text) => Err(io::Error::other(text)),
        }
    }

    /// `ScreenSource::capture`: CPU pixels; a GPU picture only read back
    /// (its read-back asks the stream for shared memory).
    pub(super) fn capture_cpu(&mut self, keyframe: bool) -> io::Result<SourceFrame> {
        self.capture_gpu(keyframe)?.into_source_frame()
    }

    /// `ScreenSource::capture_snapshot`: on the DMA-BUF route the newest
    /// picture read back once, exactly, and the stream stays on DMA-BUF;
    /// otherwise the CPU capture. Fitted into the contract as the trait's
    /// default does.
    pub(super) fn snapshot(&mut self, contract: Option<FrameContract>) -> io::Result<SourceFrame> {
        if let Some(contract) = contract {
            contract.validate()?;
        }
        let limits = ScreenSource::frame_limits_for_contract(self, contract);
        let (still, gpu_active) = match (self.stream()?, self.capture_grant()) {
            (Stream::Running(latest), Some(grant)) => {
                let active = latest.dmabuf.dmabuf().is_some();
                (latest.still_from_gpu(&grant)?, active)
            }
            _ => (None, false),
        };
        let frame = match still {
            Some(image) => SourceFrame::Frame(image),
            None if gpu_active && !self.had_frame => {
                SourceFrame::Paused(SessionState::WaitingForPortal)
            }
            None if gpu_active => SourceFrame::Unchanged,
            None => self.capture_cpu(true)?,
        };
        let image = match frame {
            SourceFrame::Frame(image) => image,
            other => return Ok(other),
        };
        let size = limits.fit(image.size());
        limits.check(size)?;
        if size == image.size() {
            return Ok(SourceFrame::Frame(image));
        }
        image
            .downscale(size, limits, &image.grant(), CaptureOwner::ScaledPixels)
            .map(SourceFrame::Frame)
    }
}
