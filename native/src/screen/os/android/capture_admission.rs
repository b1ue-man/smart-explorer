//! Android uses the host's admission authority, including all JVM owners.
use std::{io, sync::{Arc, Mutex}};
use crate::screen::core::{
    capture_admission::{CaptureGeneration, CaptureGrant, CaptureOwner, CapturePlan, SessionAdmission, SourceKey},
    capture_plan::{CapturePhases, bgra_bytes},
    host_backend::HostOutputPlan,
    host_codec_adapters::{HostCodecPlan, HostInputPixels},
    pixel_convert::FrameSize,
};

pub(crate) const ACCESSIBILITY: u32 = 0;
pub(crate) const PROJECTION_TILES: u32 = 1;
pub(crate) const PROJECTION_HYBRID: u32 = 2;
pub(crate) const PROJECTION_SURFACE: u32 = 3;
pub(crate) const SYNTHETIC_SURFACE: u32 = 4;
pub(crate) const SYNTHETIC_PIXELS: u32 = 5;
pub(crate) const ACCESS_UNIT: u64 = 16 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct CaptureSession {state: Mutex<Option<State>>}
struct State {
    admission: Arc<SessionAdmission>, source: SourceKey,
    generation: CaptureGeneration, output: Option<HostOutputPlan>,
    current: Option<CaptureGrant>,
    begun: bool,
}
impl CaptureSession {
    pub(crate) fn attach(&self, admission: Arc<SessionAdmission>, source: SourceKey) -> io::Result<()> {
        let mut state = self.state.lock().map_err(|_| io::Error::other("Android capture state poisoned"))?;
        if state.is_some() {return Err(io::Error::other("Android capture authority already attached"));}
        *state = Some(State {admission, source, generation: CaptureGeneration::new(1), output: None, current: None,begun:false});
        Ok(())
    }
    pub(crate) fn plan(&self, size: FrameSize, kind: u32, output: &HostOutputPlan) -> io::Result<CapturePlan> {
        let mut state = self.state.lock().map_err(|_| io::Error::other("Android capture state poisoned"))?;
        let state = state.as_mut().ok_or_else(|| io::Error::other("Android capture has no session authority"))?;
        let plan = plan(&state.source, state.generation, size, kind, output)?;
        state.output = Some(output.clone());
        Ok(plan)
    }
    pub(crate) fn activate(&self, grant: CaptureGrant) -> io::Result<()> {
        let mut state = self.state.lock().map_err(|_| io::Error::other("Android capture state poisoned"))?;
        let state = state.as_mut().ok_or_else(|| io::Error::other("Android capture has no session authority"))?;
        grant.check_identity(&state.source, state.generation)?;
        state.current = Some(grant);
        Ok(())
    }
    /// A replacement grant is approved before the replacement's first resource.
    /// Outstanding buffer/callback tokens keep the old grant independently alive.
    pub(crate) fn begin(&self, size: FrameSize, kind: u32) -> io::Result<u64> {
        let mut state = self.state.lock().map_err(|_| io::Error::other("Android capture state poisoned"))?;
        let state = state.as_mut().ok_or_else(|| io::Error::other("Android capture has no session authority"))?;
        if state.current.is_none() {return Err(io::Error::new(io::ErrorKind::WouldBlock, "Android capture awaits host grant"));}
        let previous = state.output.as_ref().ok_or_else(|| io::Error::other("Android capture output is pending"))?;
        let codec = if kind <= PROJECTION_TILES || kind==SYNTHETIC_PIXELS {HostCodecPlan::Tiles} else {HostCodecPlan::OpenH264};
        let limits = previous.contract().frame_contract().map_or(
            crate::screen::core::frame_budget::FrameLimits::LEGACY, |contract| contract.limits);
        let output_size = if kind == ACCESSIBILITY {limits.fit(previous.size())} else {size};
        let output = HostOutputPlan::new(previous.contract(), output_size, codec)?;
        if !state.begun {
            let candidate=plan(&state.source,state.generation,size,kind,&output)?;
            if let Some(current)=state.current.as_ref() {
                if candidate.peak_bytes()<=current.plan().peak_bytes()
                    && candidate.owner_limits().iter().zip(current.plan().owner_limits()).all(|(wanted,held)|*wanted<=held) {
                    state.begun=true;state.output=Some(output);
                    return Ok(state.generation.value());
                }
            }
        }
        let generation = state.generation.next()?;
        let plan = plan(&state.source, generation, size, kind, &output)?;
        let grant = state.admission.authority().approve(plan)?;
        // A rejected candidate preserves the previous generation. After the
        // successful handover, its held resources remain charged individually.
        retire_grant(state.current.take());
        state.generation = generation;
        state.output = Some(output);
        state.current = Some(grant);
        state.begun=true;
        Ok(generation.value())
    }
    pub(crate) fn grant(&self, generation: u64) -> io::Result<CaptureGrant> {
        let state = self.state.lock().map_err(|_| io::Error::other("Android capture state poisoned"))?;
        let grant = state.as_ref().and_then(|state| state.current.as_ref())
            .ok_or_else(|| io::Error::new(io::ErrorKind::WouldBlock, "Android capture awaits host grant"))?;
        if generation != grant.generation().value() {return Err(io::Error::other("stale Android capture generation"));}
        Ok(grant.clone())
    }
    pub(crate) fn current(&self) -> Option<CaptureGrant> {
        self.state.lock().ok()?.as_ref()?.current.clone()
    }
    pub(crate) fn synthetic_snapshot(&self,size:FrameSize)->io::Result<CaptureGrant> {
        let mut guard=self.state.lock().map_err(|_|io::Error::other("Android capture state poisoned"))?;
        let state=guard.as_mut().ok_or_else(||io::Error::other("Android capture has no authority"))?;
        let current=state.output.as_ref().ok_or_else(||io::Error::other("Android capture output is pending"))?;
        let limits=current.contract().frame_contract().map_or(
            crate::screen::core::frame_budget::FrameLimits::LEGACY, |contract|contract.limits);
        let output=HostOutputPlan::new(current.contract(),limits.fit(current.size()),HostCodecPlan::Tiles)?;
        let candidate=plan(&state.source,state.generation,size,SYNTHETIC_PIXELS,&output)?;
        if let Some(current)=state.current.as_ref() {
            if candidate.peak_bytes()<=current.plan().peak_bytes()
                && candidate.owner_limits().iter().zip(current.plan().owner_limits()).all(|(wanted,held)|*wanted<=held) {
                return Ok(current.clone());
            }
        }
        let generation=state.generation.next()?;
        let grant=state.admission.authority().approve(plan(&state.source,generation,size,SYNTHETIC_PIXELS,&output)?)?;
        state.generation=generation;
        // A native synthetic tiles source has no Kotlin resource generation.
        // Cache its complete plan, but leave an active surface encoder's grant
        // untouched while a separate RAW snapshot is consumed.
        if state.current.as_ref().is_some_and(|current|
            current.plan().owner_limits()[CaptureOwner::NativeSurface as usize]==0) {
            retire_grant(state.current.take());
            state.current=Some(grant.clone());state.output=Some(output);
        }
        Ok(grant)
    }
    pub(crate) fn retire(&self) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(mut old)=state.take() {retire_grant(old.current.take());}
        }
    }
}

fn retire_grant(grant:Option<CaptureGrant>) {
    if let Some(grant)=grant {
        if !grant.is_member() {
            if let Err(error)=grant.retire() {eprintln!("Android capture retirement: {error}");}
        }
    }
}

pub(crate) fn plan(source: &SourceKey, generation: CaptureGeneration, size: FrameSize,
    kind: u32, output: &HostOutputPlan) -> io::Result<CapturePlan> {
    use CaptureOwner as O;
    if kind > SYNTHETIC_PIXELS {return Err(io::Error::other("unknown Android capture kind"));}
    crate::screen::core::frame_budget::FrameLimits::RESOURCE.check(size)?;
    let n = bgra_bytes(size)?;
    let mut plan = CapturePlan::new(source.clone(), generation);
    plan.bind_contract(output.contract().frame_contract())?;
    let packet = n.checked_add(if kind==PROJECTION_TILES {8} else {20})
        .and_then(|n|if kind==PROJECTION_HYBRID {n.checked_add(ACCESS_UNIT)} else {Some(n)})
        .ok_or_else(|| io::Error::other("Android packet size overflow"))?;
    let mut put = |owner, bytes, count| plan.add_bytes(owner, bytes, count, CapturePhases::ALL).map(|_| ());
    match kind {
        ACCESSIBILITY => {
            // Hardware result + software bitmap, admitted before takeScreenshot.
            put(O::NativeSurface, n, 1)?;
            put(O::Readback, n, 1)?;
            // Accessibility has one outstanding request. Its native picture
            // is consumed before the next request; producer + native is 2 J.
            put(O::Transport, n.checked_add(8).ok_or_else(|| io::Error::other("Android still overflow"))?, 2)?;
            put(O::SourcePixels, n, 1)?;
        }
        PROJECTION_TILES | PROJECTION_HYBRID => {
            // maxImages is acquired images, NOT the undocumented BufferQueue.
            put(O::NativeSurface, n, 3)?;
            // Producer, latest, encoder. References to the same buffer are aliases.
            put(O::SourcePixels, n, 3)?;
            put(O::Readback, n, 1)?; // Native RGBA -> BGRA decode.
            put(O::Transport, packet, 3)?; // Producer, queued, consumer packet.
        }
        SYNTHETIC_PIXELS => {put(O::SourcePixels,n,1)?;}
        _ => {
            // The requested Canvas/Codec surface, no invented CPU RGBA copy.
            put(O::NativeSurface, n, 1)?;
            put(O::Transport, ACCESS_UNIT, 2)?;
        }
    }
    if (PROJECTION_HYBRID..=SYNTHETIC_SURFACE).contains(&kind) {
        // Partial AU, SPS/PPS, joined AU. Native codec pools are opaque OS owners.
        put(O::CodecOutput, ACCESS_UNIT, 3)?;
    }
    put(O::Metadata, 256 * 1024, 1)?;
    output.append_to(&mut plan, if kind == ACCESSIBILITY || size != output.size() {
        HostInputPixels::SeparateImage
    } else {HostInputPixels::BorrowedSource})?;
    Ok(plan)
}
