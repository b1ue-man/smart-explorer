//! Metadata planning for one authorized, serially captured window family.
use super::*;
use crate::screen::core::capture_admission::SessionAdmission;
use crate::screen::core::capture_plan::{bgra_bytes, CapturePhases};
use std::sync::atomic::AtomicBool;

pub(crate) type PrepareMember = Arc<
    dyn Fn(NativeWindowId, Arc<SessionAdmission>) -> io::Result<Box<dyn PreparedSource>>
        + Send
        + Sync,
>;

pub(super) struct PreparedMember {
    pub identity: WindowIdentity,
    pub source: Box<dyn PreparedSource>,
}

pub(super) struct PreparedSet {
    pub boundary: CaptureBoundary,
    pub members: Vec<PreparedMember>,
    pub generation: CaptureGeneration,
}

pub(super) fn surface_bytes(boundary: &CaptureBoundary) -> io::Result<u64> {
    let headers = (boundary.members.capacity() as u64)
        .checked_mul(std::mem::size_of::<crate::remote_window::CaptureMember>() as u64)
        .and_then(|bytes| bytes.checked_add(boundary.session.capacity() as u64))
        .ok_or_else(|| io::Error::other("family boundary metadata overflow"))?;
    boundary.members.iter().try_fold(headers, |bytes, member| {
        bytes
            .checked_add(std::mem::size_of::<super::members::Surface>() as u64)
            .and_then(|bytes| bytes.checked_add(member.identity.session.capacity() as u64))
            .and_then(|bytes| bytes.checked_add(member.identity.session.len() as u64))
            .and_then(|bytes| bytes.checked_add(member.binding.birth.capacity() as u64))
            .ok_or_else(|| io::Error::other("family surface metadata overflow"))
    })
}

pub(super) fn describe(
    boundary: &CaptureBoundary,
    generation: CaptureGeneration,
    limits: FrameLimits,
    fallback: FrameSize,
) -> PreparedSourceInfo {
    PreparedSourceInfo {
        native_size: Some(boundary.bounds().map_or(fallback, |bounds| {
            FrameSize::new(bounds.width, bounds.height)
        })),
        capture_limits: limits,
        supports_regions: true,
        state: if boundary.members.is_empty() {
            SessionState::Starting
        } else {
            SessionState::Streaming
        },
        generation,
    }
}

pub(super) fn prepare(
    boundary: CaptureBoundary,
    generation: CaptureGeneration,
    output: &HostOutputPlan,
    admission: &Arc<SessionAdmission>,
    open: &PrepareMember,
    changed: Arc<AtomicBool>,
) -> io::Result<PreparedSet> {
    let mut members = Vec::new();
    members
        .try_reserve_exact(boundary.members.len())
        .map_err(|_| io::Error::other("family preparation metadata allocation failed"))?;
    for member in &boundary.members {
        let context = super::members::member_admission(admission, output, changed.clone())?;
        let prepared = open(member.binding.native, context)?;
        let prepared =
            crate::screen::core::host_admission::registered(prepared, admission.clone())?;
        members.push(PreparedMember {
            identity: member.identity.clone(),
            source: prepared,
        });
    }
    Ok(PreparedSet {
        boundary,
        members,
        generation,
    })
}

pub(super) fn plan(
    set: &PreparedSet,
    key: &SourceKey,
    output: &HostOutputPlan,
    limits: FrameLimits,
) -> io::Result<Option<CapturePlan>> {
    if set.boundary.members.is_empty() {
        return Ok(None);
    }
    let bounds = set
        .boundary
        .bounds()
        .ok_or_else(|| io::Error::other("family capture bounds unavailable"))?;
    let size = FrameSize::new(bounds.width, bounds.height);
    limits.check(size)?;
    let pixels = set
        .boundary
        .members
        .iter()
        .try_fold(0u64, |total, member| {
            let size = FrameSize::new(member.geometry.width, member.geometry.height);
            limits.check(size)?;
            total
                .checked_add(size.pixels())
                .ok_or_else(|| io::Error::other("family member pixel sum overflow"))
        })?;
    // Preserve the existing family range before any platform pool is opened.
    if pixels
        > limits
            .max_pixels
            .min(limits.working_bytes / crate::screen::core::frame_budget::WORKING_IMAGES / 4)
    {
        return Err(io::Error::other("window_family_capture_budget_exceeded"));
    }
    let mut plan = CapturePlan::new(key.clone(), set.generation);
    plan.bind_contract(output.contract().frame_contract())?;
    for member in &set.members {
        let info = member.source.description();
        let Some(size) = info.native_size else {
            return Ok(None);
        };
        let source_output =
            HostOutputPlan::for_source(output.contract(), size, info.capture_limits)?;
        let Some(native) = member.source.capture_source_plan(&source_output)? else {
            return Ok(None);
        };
        if native.generation() != info.generation {
            return Ok(None);
        }
        let (persistent, transient) = split_member(native)?;
        plan.add_serial_member(persistent, transient)?;
    }
    plan.add_bytes(
        CaptureOwner::CompositionPixels,
        bgra_bytes(size)?,
        2,
        CapturePhases::ALL,
    )?;
    plan.add_bytes(
        CaptureOwner::Metadata,
        surface_bytes(&set.boundary)?,
        1,
        CapturePhases::ALL,
    )?;
    let sort = (set.boundary.members.len() as u64)
        .checked_mul(std::mem::size_of::<usize>() as u64)
        .ok_or_else(|| io::Error::other("family sort metadata overflow"))?;
    plan.add_bytes(CaptureOwner::Metadata, sort, 1, CapturePhases::ALL)?;
    output.append_to(
        &mut plan,
        crate::screen::core::capture_source_contract::HostInputPixels::BorrowedSource,
    )?;
    Ok(Some(plan))
}

/// Each source's cache aliases the retained member image. Exactly one new
/// SourcePixels allocation runs in the serial capture lane; all native pools,
/// readback resources and remaining source slots remain simultaneously live.
fn split_member(native: CapturePlan) -> io::Result<(CapturePlan, CapturePlan)> {
    let mut persistent = CapturePlan::new(native.source().clone(), native.generation());
    let mut transient = CapturePlan::new(native.source().clone(), native.generation());
    if let Some(contract) = native.contract_binding() {
        persistent.bind_contract(contract)?;
        transient.bind_contract(contract)?;
    }
    let mut producer = false;
    for requirement in native.requirements() {
        let take = requirement.owner() == CaptureOwner::SourcePixels && !producer;
        let count = requirement.count() - u64::from(take);
        if count != 0 {
            persistent.add_bytes(
                requirement.owner(),
                requirement.bytes_each(),
                count,
                requirement.phases(),
            )?;
        }
        if take {
            producer = true;
            transient.add_bytes(
                requirement.owner(),
                requirement.bytes_each(),
                1,
                requirement.phases(),
            )?;
        }
    }
    if !producer {
        return Err(io::Error::other(
            "family member plan has no CPU producer slot",
        ));
    }
    Ok((persistent, transient))
}

struct PlanState {
    set: Option<planning::PreparedSet>,
    output: Option<HostOutputPlan>,
    generation: CaptureGeneration,
}

pub(crate) struct PreparedFamily {
    host: SharedHost,
    authority: SharedAuthority,
    identity: WindowIdentity,
    key: SourceKey,
    open: PrepareMember,
    admission: Arc<SessionAdmission>,
    limits: FrameLimits,
    fallback: FrameSize,
    changed: Arc<AtomicBool>,
    state: Mutex<PlanState>,
}

impl PreparedFamily {
    pub(crate) fn new(
        host: SharedHost,
        authority: SharedAuthority,
        identity: WindowIdentity,
        open: PrepareMember,
        admission: Arc<SessionAdmission>,
        limits: FrameLimits,
    ) -> io::Result<Self> {
        let boundary = lock(&host)?.boundary(&identity, authority.as_ref())?;
        let fallback = match boundary.bounds() {
            Some(bounds) => FrameSize::new(bounds.width, bounds.height),
            None => lock(&host)?
                .snapshot(authority.as_ref())?
                .members
                .iter()
                .find(|member| member.identity == identity)
                .map(|member| FrameSize::new(member.geometry.width, member.geometry.height))
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "original_window_gone"))?,
        };
        limits.check(fallback)?;
        let key = SourceKey::new(admission.session(), &format!("family:{}", identity.session))?;
        Ok(Self {
            host,
            authority,
            identity,
            key,
            open,
            admission,
            limits,
            fallback,
            changed: Arc::new(AtomicBool::new(false)),
            state: Mutex::new(PlanState {
                set: None,
                output: None,
                generation: CaptureGeneration::new(1),
            }),
        })
    }
}

impl PreparedSource for PreparedFamily {
    fn source_key(&self) -> &SourceKey {
        &self.key
    }

    fn description(&self) -> PreparedSourceInfo {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &state.set {
            Some(set) => {
                planning::describe(&set.boundary, set.generation, self.limits, self.fallback)
            }
            None => PreparedSourceInfo {
                native_size: Some(self.fallback),
                capture_limits: self.limits,
                supports_regions: true,
                state: SessionState::Starting,
                generation: state.generation,
            },
        }
    }

    fn capture_plan(&self, output: &HostOutputPlan) -> io::Result<Option<CapturePlan>> {
        let boundary = lock(&self.host)?.boundary(&self.identity, self.authority.as_ref())?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("family plan poisoned"))?;
        if state
            .set
            .as_ref()
            .is_none_or(|set| set.boundary != boundary)
        {
            if state.set.is_some() {
                state.generation = state.generation.next()?;
            }
            state.set = Some(planning::prepare(
                boundary,
                state.generation,
                output,
                &self.admission,
                &self.open,
                self.changed.clone(),
            )?);
        }
        state.output = Some(output.clone());
        planning::plan(
            state
                .set
                .as_ref()
                .ok_or_else(|| io::Error::other("family plan missing"))?,
            &self.key,
            output,
            self.limits,
        )
    }

    fn activate(
        self: Box<Self>,
        grant: CaptureGrant,
    ) -> Result<Box<dyn ScreenSource>, SourceRefusal> {
        let state = self
            .state
            .into_inner()
            .map_err(|_| refusal(io::Error::other("family plan poisoned")))?;
        let set = state
            .set
            .ok_or_else(|| refusal(io::Error::other("family plan missing")))?;
        grant
            .check_identity(&self.key, set.generation)
            .map_err(refusal)?;
        if !lock(&self.host)
            .map_err(refusal)?
            .boundary_current(&set.boundary, self.authority.as_ref())
            .map_err(refusal)?
        {
            return Err(refusal(io::Error::new(
                io::ErrorKind::WouldBlock,
                "family layout changed before activation",
            )));
        }
        let output = state
            .output
            .ok_or_else(|| refusal(io::Error::other("family output missing")))?;
        let size = planning::describe(&set.boundary, set.generation, self.limits, self.fallback)
            .native_size
            .unwrap_or(self.fallback);
        let surfaces = members::Surfaces::activate(set, &grant).map_err(refusal)?;
        Ok(Box::new(FamilyScreenSource {
            host: self.host,
            authority: self.authority,
            identity: self.identity,
            key: self.key,
            open: self.open,
            admission: self.admission,
            limits: self.limits,
            size,
            output,
            changed: self.changed,
            surfaces: Some(surfaces),
            generation: grant.generation(),
            grant: Some(grant),
            last_revision: None,
            reference_revision: None,
            reference_generation: 1,
            active: true,
        }))
    }
}
