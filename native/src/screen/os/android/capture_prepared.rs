//! Consent and geometry discovery do not allocate product pixels.
use super::{
    bridge_channels::{self as channels, Channel, Phase},
    bridge_source::BridgeSource,
};
use crate::screen::core::{
    capture_admission::{CaptureGeneration, CaptureGrant, CapturePlan, SourceKey},
    frame_budget::FrameLimits,
    host_backend::{
        HostOutputPlan, PreparedSource, PreparedSourceInfo, ScreenSource, SourceRefusal,
    },
    pixel_convert::FrameSize,
    session_state::SessionState,
};
use std::{io, sync::Arc};
pub(crate) struct AndroidPrepared {
    channel: Arc<Channel>,
    source: SourceKey,
    limits: FrameLimits,
    activated: bool,
}
impl AndroidPrepared {
    pub(crate) fn new(channel: Arc<Channel>, source: SourceKey, limits: FrameLimits) -> Self {
        Self {
            channel,
            source,
            limits,
            activated: false,
        }
    }
    fn kind(&self, output: &HostOutputPlan) -> u32 {
        use super::android_capture_admission::*;
        if self.channel.start.source == "synthetic" {
            if output.codec() == crate::screen::core::host_codec_adapters::HostCodecPlan::Tiles {
                SYNTHETIC_PIXELS
            } else {
                SYNTHETIC_SURFACE
            }
        } else if matches!(
            self.channel.phase(),
            Phase::Ready {
                projection: false,
                ..
            }
        ) {
            ACCESSIBILITY
        } else if output.codec() == crate::screen::core::host_codec_adapters::HostCodecPlan::Tiles {
            PROJECTION_TILES
        } else if output
            .contract()
            .frame_contract()
            .is_some_and(|contract| contract.hybrid_regions)
        {
            PROJECTION_HYBRID
        } else {
            PROJECTION_SURFACE
        }
    }
}
impl PreparedSource for AndroidPrepared {
    fn source_key(&self) -> &SourceKey {
        &self.source
    }
    fn description(&self) -> PreparedSourceInfo {
        let (native_size, state) = match self.channel.phase() {
            Phase::Ready { width, height, .. } if width != 0 && height != 0 => {
                (Some(FrameSize::new(width, height)), SessionState::Starting)
            }
            Phase::Ended(_) => (None, SessionState::NoDisplay),
            _ => (None, SessionState::WaitingForPortal),
        };
        PreparedSourceInfo {
            native_size,
            capture_limits: self.limits,
            supports_regions: self.channel.start.source != "synthetic",
            state,
            generation: CaptureGeneration::new(1),
        }
    }
    fn capture_plan(&self, output: &HostOutputPlan) -> io::Result<Option<CapturePlan>> {
        let Some(size) = self.description().native_size else {
            return Ok(None);
        };
        self.channel
            .capture
            .plan(size, self.kind(output), output)
            .map(Some)
    }
    fn activate(
        mut self: Box<Self>,
        grant: CaptureGrant,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        let info = self.description();
        info.check_grant(&self.source, &grant)
            .map_err(|e| SourceRefusal::new(&e.to_string()))?;
        self.channel
            .capture
            .activate(grant)
            .map_err(|e| SourceRefusal::new(&e.to_string()))?;
        let size = info
            .native_size
            .ok_or_else(|| SourceRefusal::new("capture_metadata_pending"))?;
        self.activated = true;
        Ok(Box::new(BridgeSource::new(self.channel.clone(), size)))
    }
}
impl Drop for AndroidPrepared {
    fn drop(&mut self) {
        if !self.activated {
            channels::close(self.channel.id, "capture_preparation_cancelled");
        }
    }
}
