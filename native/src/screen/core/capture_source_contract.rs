//! Metadata-only preparation boundary. Implementations recheck permission and
//! source identity on activation, before their first native allocation.
use std::io;
use super::capture_admission::{CaptureGeneration, CaptureGrant, CapturePlan, SourceKey};
use super::frame_budget::FrameLimits;
use super::host_backend::{ScreenSource, SourceRefusal};
pub(crate) use super::host_codec_adapters::{HostCodecPlan, HostContract, HostInputPixels, HostOutputPlan};
use super::pixel_convert::FrameSize;
use super::session_state::SessionState;

#[derive(Clone, Copy, Debug)]
pub(crate) struct PreparedSourceInfo {
    /// Unknown until actual native pixel metadata exists; portal logical
    /// coordinates must not be substituted for this value.
    pub native_size: Option<FrameSize>,
    pub capture_limits: FrameLimits,
    pub supports_regions: bool,
    pub state: SessionState,
    pub generation: CaptureGeneration,
}
impl PreparedSourceInfo {
    pub(crate) fn check_grant(&self, source: &SourceKey, grant: &CaptureGrant) -> io::Result<()> {
        self.capture_limits.validate_capture()?;
        let size = self.native_size.ok_or_else(|| io::Error::other("capture native pixel metadata is pending"))?;
        super::capture_plan::bgra_bytes(size)?;
        grant.check_identity(source, self.generation)
    }
}
pub(crate) trait PreparedSource: Send {
    fn source_key(&self) -> &SourceKey;
    fn description(&self) -> PreparedSourceInfo;
    /// None means AwaitingGrant metadata, never permission to activate.
    fn capture_plan(&self, output: &HostOutputPlan) -> io::Result<Option<CapturePlan>>;
    fn activate(self: Box<Self>, grant: CaptureGrant) -> Result<Box<dyn ScreenSource>, SourceRefusal>;
}
