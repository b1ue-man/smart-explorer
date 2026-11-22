//! The read of one Graphics Capture frame and the answer without one
//! (split from `capture_wgc.rs`, review 2026-11-19 round 9).
use super::*;

impl WgcCapture {
    /// The last picture again for a key frame, else unchanged.
    pub(super) fn again(&self, keyframe: bool) -> SourcePicture {
        match &self.last {
            Some(last) if keyframe => SourcePicture::Frame(last.clone()),
            Some(_) => SourcePicture::Unchanged,
            None => SourcePicture::Paused(SessionState::Starting),
        }
    }
    /// The picture of `frame`, or `Err(size)` with its `ContentSize` when
    /// that differs from the pool.
    pub(super) fn read(
        &mut self,
        frame: &Direct3D11CaptureFrame,
    ) -> io::Result<Result<Picture, FrameSize>> {
        let content = frame.ContentSize().map_err(io::Error::other)?;
        if content.Width <= 0 || content.Height <= 0 {
            return Err(io::Error::other("capture frame has no content"));
        }
        let size = FrameSize::new(content.Width as u32, content.Height as u32);
        if self.live.as_ref().map(|live| live.size) != Some(size) {
            return Ok(Err(size));
        }
        let surface = frame.Surface().map_err(io::Error::other)?;
        let access: IDirect3DDxgiInterfaceAccess = surface.cast().map_err(io::Error::other)?;
        // SAFETY: a WGC surface is a D3D11 texture of this frame's device.
        let texture: ID3D11Texture2D =
            unsafe { access.GetInterface() }.map_err(io::Error::other)?;
        // `texture` is a BGRA texture on `self.target.devices.device`, the
        // device the texture encoder shares (`MFT_MESSAGE_SET_D3D_MANAGER`);
        // it stays valid until `frame.Close()` in `capture_waiting`. While a
        // texture encoder is armed the picture stays on the GPU (BK2a, BK3b).
        let devices = &self.target.devices;
        let read = self
            .readback
            .read(devices, &texture, size.width, size.height, &self.budget)?;
        Ok(Ok(match read {
            Read::Cpu(image) => Picture::Cpu(image.freeze()),
            Read::Gpu(picture) => Picture::Gpu(picture),
        }))
    }
}
