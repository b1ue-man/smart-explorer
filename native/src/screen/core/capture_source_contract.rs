//! Preparation never authorizes image production. A registered preparation may
//! admit an exact external NativeSurface import through CaptureImportNeed before
//! frame binding; pixels and codecs still require the final activation grant.
//! Implementations recheck permission and source identity at activation.
use super::capture_admission::{CaptureGeneration, CaptureGrant, CapturePlan, SourceKey};
use super::frame_budget::FrameLimits;
use super::host_backend::{ScreenSource, SourceRefusal};
pub(crate) use super::host_codec_adapters::{
    HostCodecPlan, HostContract, HostInputPixels, HostOutputPlan,
};
use super::pixel_convert::FrameSize;
use super::session_state::SessionState;
use std::io;

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
        let size = self
            .native_size
            .ok_or_else(|| io::Error::other("capture native pixel metadata is pending"))?;
        super::capture_plan::bgra_bytes(size)?;
        grant.check_identity(source, self.generation)
    }
}
pub(crate) trait PreparedSource: Send {
    fn source_key(&self) -> &SourceKey;
    fn description(&self) -> PreparedSourceInfo;
    /// Metadata/input release endpoints do not authorize native pixel capture.
    fn input_sink(&self) -> Option<std::sync::Arc<dyn super::host_backend::ScreenInputSink>> {
        None
    }
    /// Pure metadata preparations have no producer to pause. A preparation
    /// holding an external native stream must override this acknowledgement.
    fn set_capture_active(&mut self, _active: bool) -> io::Result<()> {
        Ok(())
    }
    /// None means AwaitingGrant metadata, never permission to activate.
    fn capture_plan(&self, output: &HostOutputPlan) -> io::Result<Option<CapturePlan>>;
    fn capture_source_plan(&self, output: &HostOutputPlan) -> io::Result<Option<CapturePlan>> {
        self.capture_plan(&output.source_only())
    }
    fn activate(
        self: Box<Self>,
        grant: CaptureGrant,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal>;
}
