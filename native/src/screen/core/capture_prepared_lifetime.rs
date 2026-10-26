//! A prepared-source registration ends after its actual prepared/active source.
use super::*;
use crate::screen::core::capture_admission::{PreparedSourceOwnerId, SourceKey};
use crate::screen::core::capture_source_contract::PreparedSourceInfo;
use crate::screen::core::host_backend::SourceRefusal;

struct Registration {
    source: SourceKey,
    owner: PreparedSourceOwnerId,
    admission: Arc<SessionAdmission>,
}
impl Drop for Registration {
    fn drop(&mut self) {
        self.admission
            .authority()
            .release_prepared_source(&self.source, self.owner);
    }
}

pub(crate) fn registered(
    prepared: Box<dyn PreparedSource>,
    admission: Arc<SessionAdmission>,
) -> io::Result<Box<dyn PreparedSource>> {
    let source = prepared.source_key().clone();
    let owner = admission.authority().own_prepared_source(source.clone())?;
    Ok(Box::new(RegisteredPrepared {
        prepared,
        registration: Registration {
            source,
            owner,
            admission,
        },
    }))
}

/// Begins the owned preparation before an OS metadata constructor can import
/// native backing. Failure destroys that constructor's resources before its ID.
pub(crate) fn prepare_registered(
    source: SourceKey,
    admission: Arc<SessionAdmission>,
    prepare: impl FnOnce() -> Result<Box<dyn PreparedSource>, SourceRefusal>,
) -> Result<Box<dyn PreparedSource>, SourceRefusal> {
    if source.session() != admission.session() {
        return Err(SourceRefusal::new("prepared owner session mismatch"));
    }
    let owner = admission
        .authority()
        .own_prepared_source(source.clone())
        .map_err(|error| SourceRefusal::new(&error.to_string()))?;
    let registration = Registration {
        source,
        owner,
        admission,
    };
    let prepared = prepare()?;
    if prepared.source_key() != &registration.source {
        return Err(SourceRefusal::new(
            "prepared constructor changed source identity",
        ));
    }
    Ok(Box::new(RegisteredPrepared {
        prepared,
        registration,
    }))
}

struct RegisteredPrepared {
    prepared: Box<dyn PreparedSource>,
    registration: Registration,
}
impl PreparedSource for RegisteredPrepared {
    fn source_key(&self) -> &SourceKey {
        self.prepared.source_key()
    }
    fn description(&self) -> PreparedSourceInfo {
        self.prepared.description()
    }
    fn input_sink(&self) -> Option<Arc<dyn ScreenInputSink>> {
        self.prepared.input_sink()
    }
    fn set_capture_active(&mut self, active: bool) -> io::Result<()> {
        self.prepared.set_capture_active(active)
    }
    fn capture_plan(
        &self,
        output: &HostOutputPlan,
    ) -> io::Result<Option<super::super::capture_plan::CapturePlan>> {
        self.prepared.capture_plan(output)
    }
    fn capture_source_plan(
        &self,
        output: &HostOutputPlan,
    ) -> io::Result<Option<super::super::capture_plan::CapturePlan>> {
        self.prepared.capture_source_plan(output)
    }
    fn activate(
        self: Box<Self>,
        grant: CaptureGrant,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        let Self {
            prepared,
            registration,
        } = *self;
        let source = prepared.activate(grant)?;
        Ok(Box::new(RegisteredSource {
            source,
            _registration: registration,
        }))
    }
}

struct RegisteredSource {
    source: Box<dyn ScreenSource>,
    _registration: Registration,
}
impl ScreenSource for RegisteredSource {
    fn legacy_input_invalid(&self) -> bool {
        self.source.legacy_input_invalid()
    }
    fn capture_grant(&self) -> Option<CaptureGrant> {
        self.source.capture_grant()
    }
    fn prepare_capture(&mut self, request: &StageRequest) -> io::Result<Option<SessionState>> {
        self.source.prepare_capture(request)
    }
    fn size(&self) -> FrameSize {
        self.source.size()
    }
    fn frame_limits(&self) -> FrameLimits {
        self.source.frame_limits()
    }
    fn frame_limits_for_contract(&self, contract: Option<FrameContract>) -> FrameLimits {
        self.source.frame_limits_for_contract(contract)
    }
    fn supports_frame_regions(&self) -> bool {
        self.source.supports_frame_regions()
    }
    fn reference_generation(&self) -> u64 {
        self.source.reference_generation()
    }
    fn family_layout_revision(&self) -> Option<u64> {
        self.source.family_layout_revision()
    }
    fn input_sink(&self) -> Option<Arc<dyn ScreenInputSink>> {
        self.source.input_sink()
    }
    fn inject_input(
        &mut self,
        backend: &dyn ScreenHostBackend,
        generation: u64,
        presented: bool,
        events: &[InputEvent],
    ) -> Result<(), ControlBlock> {
        self.source
            .inject_input(backend, generation, presented, events)
    }
    fn set_capture_active(&mut self, active: bool) -> io::Result<()> {
        self.source.set_capture_active(active)
    }
    fn capture(&mut self, frame: u64, keyframe: bool) -> io::Result<SourceFrame> {
        self.source.capture(frame, keyframe)
    }
    fn capture_snapshot(
        &mut self,
        frame: u64,
        contract: Option<FrameContract>,
    ) -> io::Result<SourceFrame> {
        self.source.capture_snapshot(frame, contract)
    }
    fn host_stage(&mut self) -> Option<&mut dyn HostStage> {
        self.source.host_stage()
    }
}
