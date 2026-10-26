//! Independent admitted family surfaces; never crop a desktop screenshot.
use std::io;
use std::sync::{Arc, Mutex, MutexGuard};
use std::sync::atomic::{AtomicBool, Ordering};
use crate::remote_window::{CaptureBoundary, HostSession, NativeWindowId, WindowAdapter,
    WindowAuthority, WindowIdentity};
use crate::screen::core::capture_admission::{CaptureGeneration, CaptureGrant,
    CaptureOwner, CapturePlan, SessionAdmission, SourceKey};
use crate::screen::core::capture_image::{CaptureImageBuilder, CapturedImage};
use crate::screen::core::capture_source_contract::{HostOutputPlan, PreparedSource, PreparedSourceInfo};
use crate::screen::core::frame_budget::FrameLimits;
use crate::screen::core::host_backend::{ScreenSource, SourceFrame, SourceRefusal};
use crate::screen::core::pixel_convert::FrameSize;
use crate::screen::core::session_state::SessionState;

#[path = "family_capture_plan.rs"]
mod planning;
#[path = "family_capture_members.rs"]
mod members;

pub(crate) type SharedHost = Arc<Mutex<HostSession<Box<dyn WindowAdapter>>>>;
pub(crate) type SharedAuthority = Arc<dyn WindowAuthority + Send + Sync>;
pub(crate) use planning::{PrepareMember, PreparedFamily};

fn lock(host: &SharedHost) -> io::Result<MutexGuard<'_, HostSession<Box<dyn WindowAdapter>>>> {
    host.lock().map_err(|_| io::Error::other("window_host_state_poisoned"))
}
fn refusal(error: io::Error) -> SourceRefusal {SourceRefusal::new(&error.to_string())}

pub(crate) struct FamilyScreenSource {
    host: SharedHost,
    authority: SharedAuthority,
    identity: WindowIdentity,
    key: SourceKey,
    open: PrepareMember,
    admission: Arc<SessionAdmission>,
    limits: FrameLimits,
    size: FrameSize,
    output: HostOutputPlan,
    changed: Arc<AtomicBool>,
    surfaces: Option<members::Surfaces>,
    grant: Option<CaptureGrant>,
    generation: CaptureGeneration,
    last_revision: Option<u64>,
    reference_revision: Option<u64>,
    reference_generation: u64,
    active: bool,
}

impl FamilyScreenSource {
    fn invalidate_reference(&mut self) -> io::Result<()> {
        if self.last_revision.take().is_some() {
            self.reference_generation = self.reference_generation.checked_add(1)
                .ok_or_else(|| io::Error::other("window_capture_reference_generation_exhausted"))?;
        }
        Ok(())
    }

    fn readmit(&mut self, boundary: CaptureBoundary) -> io::Result<bool> {
        self.generation = self.generation.next()?;
        let generation = self.generation;
        // Destruction ends every native member pool before root retirement.
        drop(self.surfaces.take());
        if let Some(grant) = self.grant.take() {grant.retire()?;}
        self.changed = Arc::new(AtomicBool::new(false));
        let set = planning::prepare(boundary, generation, &self.output,
            &self.admission, &self.open, self.changed.clone())?;
        let Some(plan) = planning::plan(&set, &self.key, &self.output, self.limits)?
            else {return Ok(false);};
        let grant = self.admission.authority().approve(plan)?;
        let mut surfaces = members::Surfaces::activate(set, &grant)?;
        for surface in &mut surfaces.values {surface.source.set_capture_active(self.active)?;}
        self.surfaces = Some(surfaces);
        self.grant = Some(grant);
        Ok(true)
    }


}

impl ScreenSource for FamilyScreenSource {
    fn size(&self) -> FrameSize {self.size}
    fn frame_limits(&self) -> FrameLimits {self.limits}
    fn capture_grant(&self) -> Option<CaptureGrant> {self.grant.clone()}
    fn reference_generation(&self) -> u64 {self.reference_generation}
    fn family_layout_revision(&self) -> Option<u64> {
        self.last_revision
    }
    fn supports_frame_regions(&self) -> bool {true}

    fn set_capture_active(&mut self, active: bool) -> io::Result<()> {
        if active != self.active {self.invalidate_reference()?;}
        self.active = false;
        let mut failure = None;
        if let Some(surfaces) = &mut self.surfaces {
            for surface in &mut surfaces.values {
                if let Err(error) = surface.source.set_capture_active(active) {failure.get_or_insert(error);}
            }
            if failure.is_some() {
                for surface in &mut surfaces.values {let _ = surface.source.set_capture_active(false);}
            }
        }
        if let Some(error) = failure {return Err(error);}
        self.active = active;
        Ok(())
    }

    fn capture(&mut self, frame_id: u64, keyframe: bool) -> io::Result<SourceFrame> {
        if !self.active {return Ok(SourceFrame::Unchanged);}
        let boundary = lock(&self.host)?.boundary(&self.identity, self.authority.as_ref())?;
        if self.reference_revision != Some(boundary.revision) {
            self.reference_generation = self.reference_generation.checked_add(1)
                .ok_or_else(|| io::Error::other("window_capture_reference_generation_exhausted"))?;
            self.reference_revision = Some(boundary.revision);
        }
        if boundary.members.is_empty() {
            self.invalidate_reference()?;
            drop(self.surfaces.take());
            if let Some(grant) = self.grant.take() {grant.retire()?;}
            let snapshot = lock(&self.host)?.snapshot(self.authority.as_ref())?;
            let minimized = snapshot.members.iter().find(|member| member.identity == snapshot.root)
                .is_some_and(|member| member.minimized);
            return Ok(SourceFrame::Paused(if minimized {SessionState::WindowMinimized}
                else {SessionState::Starting}));
        }
        if self.grant.is_none() || self.surfaces.as_ref().is_none_or(|surfaces| surfaces.boundary != boundary)
            || self.changed.load(Ordering::SeqCst) {
            self.invalidate_reference()?;
            if !self.readmit(boundary.clone())? {return Ok(SourceFrame::Paused(SessionState::Starting));}
        }
        let mut changed = self.last_revision != Some(boundary.revision);
        for surface in &mut self.surfaces.as_mut().ok_or_else(|| io::Error::other("family surfaces missing"))?.values {
            match surface.source.capture(frame_id, keyframe) {
                Ok(SourceFrame::Frame(image)) => {
                    surface.grant.check_identity(image.source(),image.generation())?;
                    if image.grant().book_identity() != surface.grant.book_identity() {
                        return Err(io::Error::new(io::ErrorKind::InvalidData,"family member escaped its root grant"));
                    }
                    surface.image = Some(image);
                    changed = true;
                }
                Ok(SourceFrame::Unchanged) if surface.image.is_some() => {},
                Ok(SourceFrame::Unchanged) => return Ok(SourceFrame::Paused(SessionState::Starting)),
                Ok(SourceFrame::Paused(state)) => return Ok(SourceFrame::Paused(state)),
                Err(_) if self.changed.load(Ordering::SeqCst) => return Ok(SourceFrame::Paused(SessionState::Starting)),
                Err(error) => {
                    if !lock(&self.host)?.boundary_current(&boundary, self.authority.as_ref())? {
                        self.invalidate_reference()?;
                        return Ok(SourceFrame::Paused(SessionState::Starting));
                    }
                    return Err(error);
                }
            }
        }
        if self.changed.load(Ordering::SeqCst)
            || !lock(&self.host)?.boundary_current(&boundary, self.authority.as_ref())?
        {
            self.invalidate_reference()?;
            return Ok(SourceFrame::Paused(SessionState::Starting));
        }
        if !changed && !keyframe {return Ok(SourceFrame::Unchanged);}
        let image = match self.compose(&boundary) {
            Ok(image) => image,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(SourceFrame::Unchanged),
            Err(error) => return Err(error),
        };
        self.size = image.size();
        self.last_revision = Some(boundary.revision);
        Ok(SourceFrame::Frame(image))
    }
}

impl Drop for FamilyScreenSource {
    fn drop(&mut self) {
        drop(self.surfaces.take());
        if let Some(grant) = self.grant.take() {let _ = grant.retire();}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::core::capture_plan::CapturePhases;
    use crate::screen::core::host_codec_adapters::{HostCodecPlan,HostContract,HostInputPixels};

    struct NativePrepared {key:SourceKey,size:FrameSize}
    impl PreparedSource for NativePrepared {
        fn source_key(&self)->&SourceKey {&self.key}
        fn description(&self)->PreparedSourceInfo {
            PreparedSourceInfo {native_size:Some(self.size),capture_limits:FrameLimits::RESOURCE,
                supports_regions:true,state:SessionState::Starting,generation:CaptureGeneration::new(1)}
        }
        fn capture_plan(&self, output:&HostOutputPlan)->io::Result<Option<CapturePlan>> {
            let mut plan = CapturePlan::new(self.key.clone(),CaptureGeneration::new(1));
            plan.bind_contract(output.contract().frame_contract())?;
            plan.add_bytes(CaptureOwner::NativeSurface,10,1,CapturePhases::ALL)?;
            plan.add_bytes(CaptureOwner::Readback,20,1,CapturePhases::ALL)?;
            plan.add_bgra(CaptureOwner::SourcePixels,self.size,2,CapturePhases::ALL)?;
            output.append_to(&mut plan,HostInputPixels::BorrowedSource)?;
            Ok(Some(plan))
        }
        fn activate(self:Box<Self>,_grant:CaptureGrant)->Result<Box<dyn ScreenSource>,SourceRefusal> {
            Err(SourceRefusal::new("metadata-only family fixture"))
        }
    }

    #[test]
    fn batches_task_capture_family_source_only_members_share_one_serial_producer_lane() {
        use crate::remote_window::{CaptureGeometry,CaptureMember,WindowBinding};
        let admission = crate::screen::core::capture_test_support::session(None);
        let root = SourceKey::new(admission.session(),"family").unwrap();
        let sizes = [FrameSize::new(2,1),FrameSize::new(1,1)];
        let mut boundary = CaptureBoundary {session:"family-fixture".into(),revision:1,members:Vec::new()};
        let mut members = Vec::new();
        for (index,size) in sizes.into_iter().enumerate() {
            let identity = WindowIdentity {session:"family-fixture".into(),generation:index as u64+1};
            boundary.members.push(CaptureMember {identity:identity.clone(),
                binding:WindowBinding {native:NativeWindowId(index as u64+1),birth:vec![1]},
                geometry:CaptureGeometry {x:if index==0 {0} else {2},y:0,width:size.width,height:size.height},
                z_order:index as u32,input:true});
            members.push(planning::PreparedMember {identity,source:Box::new(NativePrepared {
                key:SourceKey::new(admission.session(),&format!("member-{index}")).unwrap(),size})});
        }
        let set = planning::PreparedSet {boundary,members,generation:CaptureGeneration::new(1)};
        let size = FrameSize::new(3,1);
        let output = HostOutputPlan::new(HostContract::Legacy,size,HostCodecPlan::Tiles).unwrap();
        let plan = planning::plan(&set,&root,&output,FrameLimits::RESOURCE).unwrap().unwrap();
        assert_eq!(plan.owner_limits()[CaptureOwner::SourcePixels as usize],8+4+8);
        assert_eq!(plan.owner_limits()[CaptureOwner::NativeSurface as usize],20);
        assert_eq!(plan.owner_limits()[CaptureOwner::Readback as usize],40);
        let mut canvas = CapturePlan::new(root,CaptureGeneration::new(1));
        canvas.add_bgra(CaptureOwner::CompositionPixels,size,2,CapturePhases::ALL).unwrap();
        output.append_to(&mut canvas,HostInputPixels::BorrowedSource).unwrap();
        assert_eq!(plan.owner_limits()[CaptureOwner::CodecOutput as usize],
            canvas.owner_limits()[CaptureOwner::CodecOutput as usize]);
        let grant = admission.authority().approve(plan).unwrap();
        let a = grant.for_member(set.members[0].source.source_key(),CaptureGeneration::new(1)).unwrap();
        let b = grant.for_member(set.members[1].source.source_key(),CaptureGeneration::new(1)).unwrap();
        let _retained_a = a.claim(CaptureOwner::SourcePixels,8).unwrap();
        let _retained_b = b.claim(CaptureOwner::SourcePixels,4).unwrap();
        let transient_a = a.claim(CaptureOwner::SourcePixels,8).unwrap();
        assert_eq!(b.claim(CaptureOwner::SourcePixels,4).err().unwrap().kind(),io::ErrorKind::WouldBlock);
        drop(transient_a);
        assert_eq!(b.claim(CaptureOwner::SourcePixels,4).unwrap().bytes(),4);
    }
}
