//! Metadata preparation stays pending until a complete exact plan is available.
//! No fallback size or bootstrap credit authorizes capture.
use super::{
    capture_admission::{CaptureGrant, SessionAdmission},
    capture_source_contract::PreparedSource,
    frame_budget::{FrameContract, FrameLimits},
    host_backend::{ScreenHostBackend, ScreenInputSink, ScreenSource, SourceFrame},
    host_codec_adapters::HostOutputPlan,
    host_stage::{HostStage, StageRequest},
    input_model::InputEvent,
    pixel_convert::FrameSize,
    session_state::SessionState,
    wire_control::ControlBlock,
};
use std::{io, sync::Arc};

pub(crate) fn pending(
    prepared: Box<dyn PreparedSource>,
    admission: Arc<SessionAdmission>,
    output: HostOutputPlan,
    allowed: Arc<dyn Fn() -> bool + Send + Sync>,
) -> Box<dyn ScreenSource> {
    Box::new(AdmittedSource {
        active: None,
        prepared: Some(prepared),
        output,
        desired_active: true,
        allowed,
        admission,
    })
}
struct AdmittedSource {
    active: Option<Box<dyn ScreenSource>>,
    prepared: Option<Box<dyn PreparedSource>>,
    output: HostOutputPlan,
    desired_active: bool,
    allowed: Arc<dyn Fn() -> bool + Send + Sync>,
    // Last: preparations, native sources and their results die before W.
    admission: Arc<SessionAdmission>,
}
impl AdmittedSource {
    fn ensure(&mut self) -> io::Result<Option<SessionState>> {
        if !(self.allowed)() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "source_permission_revoked",
            ));
        }
        if self.active.is_some() {
            return Ok(None);
        }
        let prepared = self
            .prepared
            .as_ref()
            .ok_or_else(|| io::Error::other("capture preparation was consumed"))?;
        if !self.desired_active {
            return Ok(Some(SessionState::ViewerHidden));
        }
        let info = prepared.description();
        let Some(plan) = prepared.capture_plan(&self.output)? else {
            return Ok(Some(match info.state {
                SessionState::Streaming => SessionState::Starting,
                state => state,
            }));
        };
        let current = prepared.description();
        if current.native_size.is_none() || plan.generation() != current.generation {
            return Ok(Some(if current.state == SessionState::Streaming {
                SessionState::Starting
            } else {
                current.state
            }));
        }
        let grant = self.admission.authority().approve(plan)?;
        if !(self.allowed)() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "source_permission_revoked",
            ));
        }
        let prepared = self
            .prepared
            .take()
            .ok_or_else(|| io::Error::other("capture preparation was consumed"))?;
        let source = prepared
            .activate(grant)
            .map_err(|refusal| io::Error::other(refusal.reason))?;
        if source.capture_grant().is_none() {
            return Err(io::Error::other("activated capture source has no grant"));
        }
        self.active = Some(source);
        Ok(None)
    }
}
impl ScreenSource for AdmittedSource {
    fn legacy_input_invalid(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|source| source.legacy_input_invalid())
    }
    fn capture_grant(&self) -> Option<CaptureGrant> {
        self.active
            .as_ref()
            .and_then(|source| source.capture_grant())
    }
    fn prepare_capture(&mut self, _request: &StageRequest) -> io::Result<Option<SessionState>> {
        self.ensure()
    }
    fn size(&self) -> FrameSize {
        self.active
            .as_ref()
            .map(|source| source.size())
            .or_else(|| {
                self.prepared
                    .as_ref()
                    .and_then(|source| source.description().native_size)
            })
            .unwrap_or(FrameSize::new(0, 0))
    }
    fn frame_limits(&self) -> FrameLimits {
        self.active
            .as_ref()
            .map(|source| source.frame_limits())
            .or_else(|| {
                self.prepared
                    .as_ref()
                    .map(|source| source.description().capture_limits)
            })
            .unwrap_or(FrameLimits::LEGACY)
    }
    fn supports_frame_regions(&self) -> bool {
        self.active
            .as_ref()
            .map(|source| source.supports_frame_regions())
            .or_else(|| {
                self.prepared
                    .as_ref()
                    .map(|source| source.description().supports_regions)
            })
            .unwrap_or(false)
    }
    fn reference_generation(&self) -> u64 {
        self.active
            .as_ref()
            .map_or(0, |source| source.reference_generation())
    }
    fn family_layout_revision(&self) -> Option<u64> {
        self.active
            .as_ref()
            .and_then(|source| source.family_layout_revision())
    }
    fn input_sink(&self) -> Option<Arc<dyn ScreenInputSink>> {
        self.active
            .as_ref()
            .and_then(|source| source.input_sink())
            .or_else(|| {
                self.prepared
                    .as_ref()
                    .and_then(|source| source.input_sink())
            })
    }
    fn inject_input(
        &mut self,
        backend: &dyn ScreenHostBackend,
        generation: u64,
        presented: bool,
        events: &[InputEvent],
    ) -> Result<(), ControlBlock> {
        self.active
            .as_mut()
            .ok_or(ControlBlock::ControlUnavailable)?
            .inject_input(backend, generation, presented, events)
    }
    fn set_capture_active(&mut self, active: bool) -> io::Result<()> {
        self.desired_active = active;
        if let Some(source) = self.active.as_mut() {
            source.set_capture_active(active)
        } else if let Some(prepared) = self.prepared.as_mut() {
            prepared.set_capture_active(active)
        } else {
            Err(io::Error::other("capture preparation was consumed"))
        }
    }
    fn capture(&mut self, id: u64, keyframe: bool) -> io::Result<SourceFrame> {
        if let Some(state) = self.ensure()? {
            return Ok(SourceFrame::Paused(state));
        }
        self.active
            .as_mut()
            .ok_or_else(|| io::Error::other("capture source did not activate"))?
            .capture(id, keyframe)
    }
    fn capture_snapshot(
        &mut self,
        id: u64,
        contract: Option<FrameContract>,
    ) -> io::Result<SourceFrame> {
        if let Some(state) = self.ensure()? {
            return Ok(SourceFrame::Paused(state));
        }
        self.active
            .as_mut()
            .ok_or_else(|| io::Error::other("capture source did not activate"))?
            .capture_snapshot(id, contract)
    }
    fn host_stage(&mut self) -> Option<&mut dyn HostStage> {
        match self.active.as_mut() {
            Some(source) => source.host_stage(),
            None => None,
        }
    }
}

#[path = "capture_prepared_lifetime.rs"]
mod lifetime;
pub(crate) use lifetime::{prepare_registered, registered};
