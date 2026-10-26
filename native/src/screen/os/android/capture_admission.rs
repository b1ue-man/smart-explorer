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
        let limits=previous.limits();
        let raw=matches!(kind,ACCESSIBILITY|PROJECTION_TILES|SYNTHETIC_PIXELS);
        let output_size=if raw {limits.fit(previous.size())} else {size};
        let output=previous.reconfigure(output_size,codec)?;
        if !state.begun {
            let candidate=plan(&state.source,state.generation,size,kind,&output)?;
            if let Some(current)=state.current.as_ref() {
                if !current.is_retired()? && candidate.peak_bytes()<=current.plan().peak_bytes()
                    && candidate.owner_limits().iter().zip(current.plan().owner_limits()).all(|(wanted,held)|*wanted<=held) {
                    state.begun=true;state.output=Some(output);
                    return Ok(state.generation.value());
                }
            }
        }
        let generation = state.generation.next()?;
        let plan = plan(&state.source, generation, size, kind, &output)?;
        // End the previous allocation authority before funding Q. Retirement
        // releases only unused plan capacity: every live Kotlin/native slice
        // keeps its exact bytes until that resource actually finishes. Keeping
        // current on failure also keeps the same next generation retryable.
        if let Some(current)=state.current.as_ref() {
            if !current.is_member() {current.retire()?;}
        }
        let grant = state.admission.authority().approve(plan)?;
        // A rejected candidate preserves the generation; real old owners stay
        // funded independently after a successful replacement as well.
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
    /// Activity is control metadata. A retired image authority cannot create
    /// a new slice; its exact nine-byte reply remains fundable in the same W/X
    /// bank while real old image owners are still completing.
    pub(crate) fn activity_ack(&self,data:&[u8])->io::Result<crate::screen::core::capture_bytes::AdmittedBytes> {
        if data.len()!=9 {return Err(io::Error::new(io::ErrorKind::InvalidInput,"invalid activity acknowledgement"));}
        let state=self.state.lock().map_err(|_|io::Error::other("Android capture state poisoned"))?;
        let state=state.as_ref().ok_or_else(||io::Error::other("Android capture has no session authority"))?;
        let image=state.current.as_ref().ok_or_else(||io::Error::other("Android capture awaits host grant"))?;
        if !image.is_retired()? {
            return crate::screen::core::capture_bytes::AdmittedBytes::copy(image,CaptureOwner::Transport,data);
        }
        let mut plan=CapturePlan::new(state.source.clone(),state.generation);
        plan.add_bytes(CaptureOwner::Transport,9,1,CapturePhases::ALL)?;
        let control=state.admission.authority().approve(plan)?;
        crate::screen::core::capture_bytes::AdmittedBytes::copy(&control,CaptureOwner::Transport,data)
    }
    pub(crate) fn synthetic_snapshot(&self,size:FrameSize)->io::Result<CaptureGrant> {
        let mut guard=self.state.lock().map_err(|_|io::Error::other("Android capture state poisoned"))?;
        let state=guard.as_mut().ok_or_else(||io::Error::other("Android capture has no authority"))?;
        let current=state.output.as_ref().ok_or_else(||io::Error::other("Android capture output is pending"))?;
        let limits=current.limits();
        let output=current.reconfigure(limits.fit(current.size()),HostCodecPlan::Tiles)?;
        let candidate=plan(&state.source,state.generation,size,SYNTHETIC_PIXELS,&output)?;
        if let Some(current)=state.current.as_ref() {
            if !current.is_retired()? && candidate.peak_bytes()<=current.plan().peak_bytes()
                && candidate.owner_limits().iter().zip(current.plan().owner_limits()).all(|(wanted,held)|*wanted<=held) {
                return Ok(current.clone());
            }
        }
        let generation=state.generation.next()?;
        let replace=match state.current.as_ref() {
            Some(current)=>current.is_retired()? || current.plan().owner_limits()[CaptureOwner::NativeSurface as usize]==0,
            None=>false,
        };
        if replace {
            if let Some(current)=state.current.as_ref() {
                if !current.is_member() {current.retire()?;}
            }
        }
        let grant=state.admission.authority().approve(plan(&state.source,generation,size,SYNTHETIC_PIXELS,&output)?)?;
        state.generation=generation;
        // A native synthetic tiles source has no Kotlin resource generation.
        // Cache its complete plan; a retired surface authority is replaceable
        // as well. An active surface grant stays intact for parallel snapshots.
        if replace {
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
