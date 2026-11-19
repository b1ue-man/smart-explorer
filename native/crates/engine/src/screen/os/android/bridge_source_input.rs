//! Actual still acquisition and admitted RGBA input conversion for Android.
use super::*;
use crate::screen::core::host_backend::ScreenInputSink;
use crate::screen::core::input_model::InputEvent;
use crate::screen::core::wire_control::ControlBlock;

impl BridgeSource {
    /// The still that answers `frame_id`, as BGRA.
    pub(super) fn still(
        &mut self,
        frame_id: u64,
        limits: FrameLimits,
    ) -> io::Result<CapturedImage> {
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
        let projected = matches!(
            self.channel.phase(),
            Phase::Ready {
                projection: true,
                ..
            }
        );
        // 333 ms is the minimum interval of AccessibilityService.takeScreenshot
        // (ERROR_TAKE_SCREENSHOT_INTERVAL_TIME_SHORT); the MediaProjection
        // ImageReader (acquireLatestImage) has none, the pipeline clock paces it.
        if !projected {
            if let Some(last) = self.last_still {
                let left = STILL_INTERVAL.saturating_sub(last.elapsed());
                std::thread::sleep(left);
            }
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
            if projected {
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
pub fn decode_still(
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

/// The input of one Android host session (review 2026-11-19, Codex 3/3):
/// every batch carries the session's bridge channel, so the app performs it
/// only while that session exists, and its batches still waiting are dropped
/// when it ends (`bridge_channels::close`). The checks are the backend's
/// (`BridgeBackend::inject`): the accessibility service is on and a capture
/// streams.
pub(super) struct BridgeInputSink {
    pub(super) channel: Arc<Channel>,
}

impl BridgeInputSink {
    /// `controlling`: a batch of the input path, which the session sends only
    /// while it holds control (it drops other sessions' leftovers); `false`:
    /// a release of the session's release job, performed like its input.
    fn queue(&self, events: &[InputEvent], controlling: bool) -> Result<(), ControlBlock> {
        if !super::super::bridge_channels::prereqs().accessibility_on() {
            return Err(ControlBlock::ControlUnavailable);
        }
        // Never waits (the session loop awaits this call); a full queue is a
        // reported block, never a silent loss.
        let id = self.channel.id;
        let queued = if controlling {
            super::super::bridge_channels::send_controlling_input(id, events)
        } else {
            super::super::bridge_channels::send_input(id, events)
        };
        if queued {
            Ok(())
        } else {
            Err(ControlBlock::ControlUnavailable)
        }
    }
}

impl ScreenInputSink for BridgeInputSink {
    fn inject(
        &self,
        events: &[InputEvent],
        _reference_generation: u64,
        _presented: bool,
    ) -> Result<(), ControlBlock> {
        let streaming = super::super::bridge_channels::channels()
            .iter()
            .any(|channel| matches!(channel.phase(), Phase::Ready { .. }) && !channel.start.still);
        if !streaming {
            return Err(ControlBlock::ControlUnavailable);
        }
        self.queue(events, true)
    }

    fn release(&self, events: &[InputEvent]) -> Result<(), ControlBlock> {
        self.queue(events, false)
    }

    /// The session gives the control up or ends (round 7): what of it still
    /// waits and this release only end its strokes in the app's gesture
    /// state; nothing of it is performed afterwards (Codex 6/4).
    fn revoke(&self, events: &[InputEvent]) -> Result<(), ControlBlock> {
        if !super::super::bridge_channels::prereqs().accessibility_on() {
            return Err(ControlBlock::ControlUnavailable);
        }
        if super::super::bridge_channels::send_release(self.channel.id, events) {
            Ok(())
        } else {
            Err(ControlBlock::ControlUnavailable)
        }
    }

    /// A batch the app queued but did not perform (`inputRefused`).
    fn take_refusal(&self) -> Option<ControlBlock> {
        self.channel.take_input_refused()
    }
}
