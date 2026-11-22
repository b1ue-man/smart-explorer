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
//! duplication (`stop`). Codex 10/5: a rotation on the CPU (a rotated
//! display on the CPU route) happens before the release, so its refusal
//! holds the frame as well and no longer rebuilds the duplication.
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
        mut resource: Option<IDXGIResource>,
        picture: Option<io::Result<Option<Read>>>,
        regions: Option<AdmittedRegions>,
        keyframe: bool,
    ) -> io::Result<SourcePicture> {
        let geometry = self.budget.geometry();
        // Codex 10/5: blended and rotated while the frame is still acquired.
        // Round 11: an error of `made` (regions, grant, rotation) is no lost
        // read: the reference stays, so no timed-out wait rebuilds the
        // duplication with a new generation (`capture_slots::replace`).
        let mut made_failed = false;
        let picture = match picture {
            Some(Ok(Some(read))) => {
                let made = self.made(read, regions, geometry);
                made_failed = made
                    .as_ref()
                    .is_err_and(|e| e.kind() != io::ErrorKind::WouldBlock);
                Some(made.map(Some))
            }
            Some(Ok(None)) => Some(Ok(None)),
            Some(Err(error)) => Some(Err(error)),
            None => None,
        };
        let refused = matches!(&picture, Some(Err(e)) if e.kind() == io::ErrorKind::WouldBlock);
        // Codex 10/1: the handle is taken only to be held.
        if let Some(held) = resource.take().filter(|_| refused) {
            self.held = Some(held);
            return Ok(match self.last {
                Some(_) => self.again(keyframe),
                None => SourcePicture::Unchanged, // as before
            });
        }
        // SAFETY: the frame acquired before, released once (`resource` went).
        let released = unsafe { duplication.ReleaseFrame() };
        if let Err(error) = released {
            drop(picture);
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
            Some(Ok(Some(picture))) => {
                self.size = picture.size();
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
                if !made_failed {
                    self.last = None;
                }
                Err(error)
            }
            None => Ok(self.again(keyframe)),
        }
    }

    /// The picture of `read`: the GPU route has the overlay and no rotation
    /// (the regions and the pointer belong to CPU pixels only, BK3b); CPU
    /// pixels get their regions, the blended pointer and the rotation. A
    /// rotation refused for memory is `WouldBlock`, so the frame is held and
    /// read again (Codex 10/5).
    fn made(
        &mut self,
        read: Read,
        regions: Option<AdmittedRegions>,
        geometry: super::super::capture_slots::CaptureGeometry,
    ) -> io::Result<Picture> {
        let mut builder = match read {
            Read::Gpu(picture) => return Ok(Picture::Gpu(picture)),
            Read::Cpu(builder) => builder,
        };
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
        let grant = self.budget.grant()?;
        let image = builder.freeze().rotate(
            rotation,
            geometry.limits,
            grant,
            CaptureOwner::SourcePixels,
        )?;
        Ok(Picture::Cpu(image))
    }
}
