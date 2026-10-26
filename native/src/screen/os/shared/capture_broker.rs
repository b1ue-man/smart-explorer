//! Parent-only funding. Worker and Parent owners are disjoint plans in one
//! SessionAdmission; only retirement or confirmed exit releases Worker credit.
use std::{collections::HashMap,io,sync::{Arc,Mutex}};
use crate::screen::core::{capture_admission::{CaptureGrant,CapturePlan,CaptureSlice,SessionAdmission,SourceKey,PreparedSourceOwnerId},
    capture_plan::OWNER_COUNT,host_codec_adapters::{HostInputPixels,HostOutputPlan},
    worker_capture_wire::{CreditWire,Reduction,owner}};

struct Entry {
    worker:Option<CaptureGrant>,
    parent:Option<CaptureGrant>,
    held:Option<[Option<CaptureSlice>;OWNER_COUNT]>,
}
struct PreparedOwners {manual:bool, owners:Vec<PreparedSourceOwnerId>}
struct State {
    output:Option<HostOutputPlan>,next:u64,current:Option<u64>,entries:HashMap<u64,Entry>,
    exited:bool,reader_ended:bool,prepared:HashMap<String,PreparedOwners>,prepared_root:Option<SourceKey>,
}
pub(super) struct CaptureBroker {admission:Arc<SessionAdmission>,source:SourceKey,state:Mutex<State>}
impl CaptureBroker {
    pub(super) fn new(admission:Arc<SessionAdmission>,source:SourceKey)->Arc<Self> {
        Arc::new(Self{admission,source,state:Mutex::new(State{output:None,next:1,current:None,
            entries:HashMap::new(),exited:false,reader_ended:false,prepared:HashMap::new(),prepared_root:None})})
    }
    pub(super) fn session(&self)->&str {self.admission.session()}
    pub(super) fn working_bytes(&self)->u64 {self.admission.working_bytes()}
    pub(super) fn source(&self)->&SourceKey {&self.source}
    pub(super) fn bind_source(&self,source:SourceKey)->io::Result<()> {
        if source.session()!=self.session() {return Err(io::Error::other("prepared worker source has another session"));}
        let mut state=self.state.lock().map_err(|_|io::Error::other("capture broker poisoned"))?;
        if state.prepared_root.as_ref().is_some_and(|bound|bound!=&source) {return Err(io::Error::other("worker prepared root changed"));}
        state.prepared_root=Some(source);Ok(())
    }
    pub(super) fn configure(&self,output:HostOutputPlan)->io::Result<()> {
        self.admission.bind_contract(output.contract().frame_contract())?;
        let mut state=self.state.lock().map_err(|_|io::Error::other("capture broker poisoned"))?;
        if let Some(previous)=&state.output {
            if previous.contract()!=output.contract() {return Err(io::Error::other("worker output contract changed"));}
        }
        state.output=Some(output);Ok(())
    }
    pub(super) fn contract(&self)->io::Result<Option<crate::screen::core::frame_budget::FrameContract>> {
        self.state.lock().map_err(|_|io::Error::other("capture broker poisoned"))?.output.as_ref()
            .map(|output|output.contract().frame_contract()).ok_or_else(||io::Error::other("worker output is not bound"))
    }
    pub(super) fn parent_plan(&self,worker:&CapturePlan)->io::Result<CapturePlan> {
        let state=self.state.lock().map_err(|_|io::Error::other("capture broker poisoned"))?;
        let output=state.output.as_ref().ok_or_else(||io::Error::other("worker output is not bound"))?;
        Self::make_parent(output,worker)
    }
    fn make_parent(output:&HostOutputPlan,worker:&CapturePlan)->io::Result<CapturePlan> {
        let mut plan=CapturePlan::new(worker.source().clone(),worker.generation());
        output.for_parent_of(worker).append_to(&mut plan,HostInputPixels::SeparateImage)?;Ok(plan)
    }
    pub(super) fn approve(&self,worker:CapturePlan,parent:Option<CaptureGrant>)->io::Result<CreditWire> {
        let mut state=self.state.lock().map_err(|_|io::Error::other("capture broker poisoned"))?;
        if state.prepared_root.as_ref()!=Some(worker.source()) {return Err(io::Error::new(io::ErrorKind::PermissionDenied,"worker capture source changed"));}
        if state.exited {return Err(io::Error::new(io::ErrorKind::BrokenPipe,"worker has exited"));}
        let output=state.output.as_ref().ok_or_else(||io::Error::other("worker contract is unbound"))?;
        let parent_plan=Self::make_parent(output,&worker)?;
        let parent=if let Some(parent)=parent {
            parent.check_identity(parent_plan.source(),parent_plan.generation())?;
            if parent.plan().peak_bytes()!=parent_plan.peak_bytes() || parent.plan().owner_limits()!=parent_plan.owner_limits() {
                return Err(io::Error::other("Parent capture grant differs from prepared partition"));
            }
            parent
        } else {
            if let Some(current)=state.current.and_then(|id|state.entries.get(&id)) {if let Some(parent)=&current.parent {parent.retire()?;}}
            self.admission.authority().approve(parent_plan)?
        };
        let worker=self.admission.authority().approve(worker)?;
        let id=state.next;
        state.next=id.checked_add(1).ok_or_else(||io::Error::other("worker grant ID exhausted"))?;
        let credit=CreditWire{id,source:worker.source().source().into(),generation:worker.generation().value(),
            total:worker.plan().peak_bytes(),owners:worker.plan().owner_limits()};
        state.entries.try_reserve(1).map_err(|_|io::Error::other("worker grant registry allocation failed"))?;
        state.entries.insert(id,Entry{worker:Some(worker),parent:Some(parent),held:None});state.current=Some(id);Ok(credit)
    }
    pub(super) fn grant(&self,id:u64)->io::Result<CaptureGrant> {
        self.state.lock().map_err(|_|io::Error::other("capture broker poisoned"))?.entries.get(&id)
            .and_then(|entry|entry.parent.clone()).ok_or_else(||io::Error::other("unknown or retired worker grant ID"))
    }
    pub(super) fn current(&self)->Option<CaptureGrant> {
        let state=self.state.lock().ok()?;state.current.and_then(|id|state.entries.get(&id)).and_then(|entry|entry.parent.clone())
    }
    pub(super) fn reduced(&self,reduction:Reduction)->io::Result<()> {
        let mut state=self.state.lock().map_err(|_|io::Error::other("capture broker poisoned"))?;
        if state.exited && !state.entries.contains_key(&reduction.id) {return Ok(());}
        let entry=state.entries.get_mut(&reduction.id).ok_or_else(||io::Error::other("unknown reduced worker grant"))?;
        let Some(worker)=entry.worker.as_ref() else {return Ok(());};
        if entry.held.is_none() {
            let total=reduction.owners.iter().try_fold(0u64,|n,bytes|n.checked_add(*bytes))
                .ok_or_else(||io::Error::other("retired worker owner sum overflow"))?;
            if total>worker.plan().peak_bytes() {return Err(io::Error::other("retired worker exceeded funded sum"));}
            let mut held=std::array::from_fn(|_|None);
            for (index,bytes) in reduction.owners.iter().copied().enumerate() {
                held[index]=Some(worker.claim(owner(index as u8)?,bytes)?);
            }
            worker.retire()?;entry.held=Some(held);
        } else if let Some(held)=entry.held.as_mut() {
            for (index,bytes) in reduction.owners.iter().copied().enumerate() {
                let previous=held[index].as_ref().map_or(0,CaptureSlice::bytes);
                if bytes>previous {return Err(io::Error::other("retired worker ownership increased"));}
            }
            for (index,bytes) in reduction.owners.iter().copied().enumerate() {
                if let Some(previous)=held[index].take() {
                    let (retained,released)=previous.split(bytes)?;held[index]=Some(retained);drop(released);
                }
            }
        }
        Ok(())
    }
    pub(super) fn retired(&self,id:u64)->io::Result<()> {
        let mut state=self.state.lock().map_err(|_|io::Error::other("capture broker poisoned"))?;
        if state.exited && !state.entries.contains_key(&id) {return Ok(());}
        state.entries.remove(&id).ok_or_else(||io::Error::other("unknown retired worker grant"))?;
        if state.current==Some(id) {state.current=None;}Ok(())
    }
    pub(super) fn process_exited(&self) {
        let mut state=self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.exited=true;
        for (source,prepared) in state.prepared.drain() {
            if let Ok(source) = SourceKey::new(self.session(),&source) {
                let authority = self.admission.authority();
                if prepared.manual {authority.unregister_prepared_source(&source);}
                for owner in prepared.owners {authority.release_prepared_source(&source,owner);}
            }
        }
        for entry in state.entries.values_mut() {entry.held=None;entry.worker=None;}
        state.entries.retain(|_,entry|entry.parent.is_some());
        if state.reader_ended {state.entries.clear();state.current=None;}
    }
    pub(super) fn reader_ended(&self) {
        let mut state=self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.reader_ended=true;
        if state.exited {state.entries.clear();state.current=None;}
    }
}

#[path="capture_broker_imports.rs"]
mod imports;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::core::{capture_admission::{CaptureGeneration,CaptureOwner},
        capture_plan::CapturePhases,host_codec_adapters::{HostCodecPlan,HostContract},
        pixel_convert::FrameSize};

    fn funded() -> (Arc<SessionAdmission>,Arc<CaptureBroker>,u64) {
        let admission = crate::screen::core::capture_test_support::session(None);
        let key = SourceKey::new(admission.session(),"worker").unwrap();
        let broker = CaptureBroker::new(admission.clone(),SourceKey::new(admission.session(),"synthetic").unwrap());
        broker.bind_source(key.clone()).unwrap();
        let output = HostOutputPlan::new(HostContract::Legacy,FrameSize::new(32,18),HostCodecPlan::Tiles).unwrap();
        broker.configure(output.clone()).unwrap();
        let mut plan = CapturePlan::new(key,CaptureGeneration::new(1));
        plan.add_bytes(CaptureOwner::NativeSurface,600*1024*1024,1,CapturePhases::ALL).unwrap();
        plan.add_bgra(CaptureOwner::SourcePixels,FrameSize::new(32,18),2,CapturePhases::ALL).unwrap();
        output.for_worker().append_to(&mut plan,HostInputPixels::BorrowedSource).unwrap();
        let id = broker.approve(plan,None).unwrap().id;
        (admission,broker,id)
    }

    #[test]
    fn batches_task_capture_worker_eof_does_not_release_credit_before_process_exit() {
        let (admission,broker,id) = funded();
        let extra = admission.additional_bytes().unwrap();
        assert!(extra > 0);
        broker.reader_ended();
        assert_eq!(admission.additional_bytes().unwrap(),extra);
        assert!(broker.grant(id).is_ok());
        broker.process_exited();
        assert_eq!(admission.additional_bytes().unwrap(),0);
        assert!(broker.grant(id).is_err());
    }

    #[test]
    fn batches_task_capture_worker_confirmed_exit_keeps_buffered_parent_reply_authority() {
        let (_admission,broker,id) = funded();
        broker.process_exited();
        let buffered = broker.grant(id).unwrap();
        broker.reader_ended();
        assert!(broker.grant(id).is_err());
        assert!(buffered.claim(CaptureOwner::CompositionPixels,32*18*4).is_ok());
    }

    #[test]
    fn batches_task_capture_worker_retirement_reduction_is_monotonic_until_final_ack() {
        let (admission,broker,id) = funded();
        assert!(admission.additional_bytes().unwrap() > 0);
        let mut owners = [0;OWNER_COUNT];
        owners[CaptureOwner::NativeSurface as usize] = 5*1024*1024;
        broker.reduced(Reduction {id,owners}).unwrap();
        assert_eq!(admission.additional_bytes().unwrap(),0);
        let mut enlarged = owners;
        enlarged[CaptureOwner::NativeSurface as usize] += 1;
        assert!(broker.reduced(Reduction {id,owners:enlarged}).is_err());
        assert!(broker.grant(id).is_ok());
        broker.retired(id).unwrap();
        assert!(broker.grant(id).is_err());
    }
}
