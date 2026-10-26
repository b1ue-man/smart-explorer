//! Actual still acquisition and admitted RGBA input conversion for Android.
use super::*;

impl BridgeSource {
    /// The still that answers `frame_id`, as BGRA.
    pub(super) fn still(&mut self, frame_id: u64, limits: FrameLimits) -> io::Result<CapturedImage> {
        if self.synthetic() {
            let grant = self.channel.capture.synthetic_snapshot(self.size)?;
            let mut image = CaptureImageBuilder::new(
                self.size,
                FrameLimits::RESOURCE,
                &grant,
                CaptureOwner::SourcePixels,
            )?;
            crate::screen::core::synthetic::render(frame_id, &mut image)?;
            let image = image.freeze();
            let target = limits.fit(image.size());
            return image.downscale(target, limits, &grant, CaptureOwner::ScaledPixels);
        }
        if let Some(last) = self.last_still {
            let left = STILL_INTERVAL.saturating_sub(last.elapsed());
            std::thread::sleep(left);
        }
        self.channel
            .send(json!({"op": "still", "frameId": frame_id}));
        self.last_still = Some(Instant::now());
        let picture = self
            .channel
            .next_picture(STILL_TIMEOUT, Picture::still)
            .map_err(io::Error::other)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "no still from the app"))?;
        let grant = picture.data.grant()?;
        let image = decode_still(
            picture.data.as_slice(),
            &grant,
            if matches!(
                self.channel.phase(),
                Phase::Ready {
                    projection: true,
                    ..
                }
            ) {
                CaptureOwner::Readback
            } else {
                CaptureOwner::SourcePixels
            },
        )?;
        let target = limits.fit(image.size());
        if target == image.size() {
            limits.check(target)?;
            Ok(image)
        } else {
            image.downscale(target, limits, &grant, CaptureOwner::ScaledPixels)
        }
    }

}

/// A still of the app: width and height (u32 little endian), RGBA rows.
pub(crate) fn decode_still(
    data: &[u8],
    grant: &CaptureGrant,
    owner: CaptureOwner,
) -> io::Result<CapturedImage> {
    let invalid = |text: &str| io::Error::new(io::ErrorKind::InvalidData, text.to_string());
    let header = data
        .get(..STILL_HEADER)
        .ok_or_else(|| invalid("still without header"))?;
    let word = |at: usize| {
        u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
    };
    let size = FrameSize::new(word(0), word(4));
    let len = FrameLimits::RESOURCE.check(size)?;
    let pixels = &data[STILL_HEADER..];
    if pixels.len() != len {
        return Err(invalid("still size does not match its pixels"));
    }
    let mut bgra = AdmittedBytes::copy(grant, owner, pixels)?;
    for pixel in bgra.as_mut_slice().chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 0xFF;
    }
    Ok(CaptureImageBuilder::from_bytes(size, FrameLimits::RESOURCE, grant, bgra)?.freeze())
}
