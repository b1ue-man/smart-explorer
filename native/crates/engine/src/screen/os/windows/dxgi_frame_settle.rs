//! The end of one Desktop Duplication frame: its read becomes the picture,
//! the frame is released (split from `capture_dxgi.rs`, review 2026-11-19
//! round 9).
//!
//! H2: a read refused for memory (`WouldBlock`: the encoder still holds a
//! picture of the capture plan, also past `READ_WAIT`) keeps the frame
//! acquired (`DxgiCapture::held`) and the reference picture (`last`); the
//! next capture call reads that frame again, as a whole picture (its dirty
//! rectangles went with the refusal), before it acquires another. Before,
//! the reference was dropped before the read: the next timed-out wait
//! rebuilt the duplication, with a new capture generation and so a new
//! encoder, however long the encoding took. A held frame goes with the
//! duplication (`stop`). The only frame still lost is one whose rotation on
//! the CPU is refused for memory after its release (a rotated display on the
//! CPU route): its reference goes, as before.
//!
//! H4: the picture records the pointer mode it was read in
//! (`DxgiCapture::blends_now`, taken once before the read), not the mode a
//! switch set while it was read.
use super::*;
use crate::screen::core::capture_image::AdmittedRegions;

impl DxgiCapture {
    /// The held frame read again (see the module text).
    pub(super) fn read_held(
        &mut self,
        duplication: &IDXGIOutputDuplication,
        resource: IDXGIResource,
        release: &mut dyn FnMut() -> bool,
        keyframe: bool,
    ) -> io::Result<SourcePicture> {
        // The whole picture is read in the current pointer mode.
        self.switched_at = None;
        release();
        let picture = Some(self.read_waiting(&resource));
        self.settle_frame(duplication, Some(resource), picture, None, keyframe)
    }

    /// Releases the frame of `resource` (or holds it, H2) and makes the
    /// picture of its read.
    pub(super) fn settle_frame(
        &mut self,
        duplication: &IDXGIOutputDuplication,
        resource: Option<IDXGIResource>,
        picture: Option<io::Result<Option<Read>>>,
        regions: Option<AdmittedRegions>,
        keyframe: bool,
    ) -> io::Result<SourcePicture> {
        let refused = matches!(&picture, Some(Err(e)) if e.kind() == io::ErrorKind::WouldBlock);
        if let Some(resource) = resource.filter(|_| refused) {
            self.held = Some(resource);
            return Ok(match self.last {
                Some(_) => self.again(keyframe),
                None => SourcePicture::Unchanged, // as before
            });
        }
        let geometry = self.budget.geometry();
        drop(resource);
        // SAFETY: the frame acquired before, released once.
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
                self.last_blends = self.blends_now;
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
                    let pixels = builder.pixels_mut();
                    let blends = self.blends_now;
                    pointer.blend_into(pixels, delivered, geometry.native, rotation, blends);
                }
                let image = builder.freeze();
                let image = match image.rotate(
                    rotation,
                    geometry.limits,
                    self.budget.grant()?,
                    CaptureOwner::SourcePixels,
                ) {
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        // The frame is released: a whole picture only from
                        // a new duplication (see the module text).
                        self.last = None;
                        self.first.missed();
                        return Ok(SourcePicture::Unchanged);
                    }
                    result => result?,
                };
                self.size = image.size();
                let picture = Picture::Cpu(image);
                self.last = Some(picture.clone());
                self.last_blends = self.blends_now;
                self.first.delivered();
                Ok(SourcePicture::Frame(picture))
            }
            Some(Ok(None)) => {
                self.stop();
                Ok(SourcePicture::Paused(SessionState::Starting))
            }
            Some(Err(error)) => {
                self.last = None;
                Err(error)
            }
            None => Ok(self.again(keyframe)),
        }
    }
}
