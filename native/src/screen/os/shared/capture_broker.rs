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
            // A previous Worker reply may already own its transport while an
            // asynchronous source admits this plan. Its Parent must still be
            // able to receive that ordered reply until RETIRED follows it.
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
        // Parent results can still need their planned snapshot/forwarding
        // allocations after the Worker ends. Their own slices retain that
        // capability until their actual downstream owners finish.
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

    #[test]
    fn batches_task_capture_integration_cursor_credit_does_not_replace_parent_image() {
        use crate::screen::core::worker_capture_wire::{GrantRequest,GrantValue,PlanWire};
        let (admission,broker,image)=funded();
        let original=broker.grant(image).unwrap();
        let before=admission.additional_bytes().unwrap();
        let mut cursor=CapturePlan::new(original.source().clone(),original.generation());
        cursor.add_bytes(CaptureOwner::Readback,80,1,CapturePhases::ALL).unwrap();
        cursor.add_bytes(CaptureOwner::Metadata,32,1,CapturePhases::ALL).unwrap();
        let request=GrantRequest::SupplementPlan {request:17,image,plan:PlanWire::new(&cursor)};
        let payload=crate::screen::core::wire::to_json(&request).unwrap();
        let request=crate::screen::core::wire::from_json(&payload).unwrap();
        let (request,credit)=broker.resource(request).unwrap();
        assert_eq!(request,17);
        let GrantValue::Supplement(credit)=credit.unwrap() else {panic!("wrong cursor credit class");};
        assert_eq!(admission.additional_bytes().unwrap(),before+112);
        assert_eq!(broker.current().unwrap().book_identity(),original.book_identity());
        assert!(broker.grant(credit.id).is_err(),"cursor has no Parent image partition");
        assert!(original.claim(CaptureOwner::Transport,64).is_ok());
        broker.retired(credit.id).unwrap();
        assert_eq!(admission.additional_bytes().unwrap(),before);
        assert!(broker.grant(image).unwrap().claim(CaptureOwner::Transport,64).is_ok());
        assert_eq!(broker.current().unwrap().book_identity(),original.book_identity());
    }

    #[test]
    fn batches_task_capture_integration_broker_checks_cursor_member_whitelist_and_generation() {
        use crate::screen::core::worker_capture_wire::{GrantRequest,PlanWire};
        let (_,broker,image)=funded();
        let original=broker.grant(image).unwrap();
        for (source,generation,owner) in [
            (SourceKey::new(original.source().session(),"foreign-member").unwrap(),original.generation(),CaptureOwner::Readback),
            (original.source().clone(),CaptureGeneration::new(99),CaptureOwner::Metadata),
            (original.source().clone(),original.generation(),CaptureOwner::CodecOutput),
        ] {
            let mut cursor=CapturePlan::new(source,generation);
            cursor.add_bytes(owner,32,1,CapturePhases::ALL).unwrap();
            let (_,result)=broker.resource(GrantRequest::SupplementPlan {request:18,image,plan:PlanWire::new(&cursor)}).unwrap();
            assert!(result.is_err());
        }
        assert!(original.claim(CaptureOwner::Transport,64).is_ok());
        assert_eq!(broker.current().unwrap().book_identity(),original.book_identity());
    }

    #[test]
    fn batches_task_capture_integration_broker_admits_the_bound_family_member_cursor() {
        use crate::screen::core::worker_capture_wire::{GrantRequest,GrantValue,PlanWire};
        let admission=crate::screen::core::capture_test_support::session(None);
        let root=SourceKey::new(admission.session(),"family").unwrap();
        let member=SourceKey::new(admission.session(),"x11-member").unwrap();
        let broker=CaptureBroker::new(admission.clone(),SourceKey::new(admission.session(),"synthetic").unwrap());
        broker.bind_source(root.clone()).unwrap();
        let output=HostOutputPlan::new(HostContract::Legacy,FrameSize::new(4,2),HostCodecPlan::Tiles).unwrap();
        broker.configure(output.clone()).unwrap();
        let mut plan=CapturePlan::new(root,CaptureGeneration::new(2));
        plan.add_bgra(CaptureOwner::CompositionPixels,FrameSize::new(4,2),2,CapturePhases::ALL).unwrap();
        let mut child=CapturePlan::new(member.clone(),CaptureGeneration::new(4));
        child.add_bgra(CaptureOwner::SourcePixels,FrameSize::new(4,2),2,CapturePhases::ALL).unwrap();
        plan.add_member(child).unwrap();
        output.for_worker().append_to(&mut plan,HostInputPixels::BorrowedSource).unwrap();
        let image=broker.approve(plan,None).unwrap();
        let parent=broker.current().unwrap();
        let mut cursor=CapturePlan::new(member,CaptureGeneration::new(4));
        cursor.add_bytes(CaptureOwner::Readback,64,1,CapturePhases::ALL).unwrap();
        cursor.add_bytes(CaptureOwner::Metadata,8,1,CapturePhases::ALL).unwrap();
        let (_,result)=broker.resource(GrantRequest::SupplementPlan {request:19,image:image.id,plan:PlanWire::new(&cursor)}).unwrap();
        let GrantValue::Supplement(cursor)=result.unwrap() else {panic!("wrong family cursor credit class");};
        assert_eq!(cursor.source,"x11-member");
        assert_eq!(cursor.generation,4);
        broker.retired(cursor.id).unwrap();
        assert_eq!(broker.current().unwrap().book_identity(),parent.book_identity());
        assert!(parent.claim(CaptureOwner::CompositionPixels,32).is_ok());
    }

    #[test]
    fn batches_task_capture_integration_new_plan_keeps_an_old_prepared_reply_receivable() {
        use crate::screen::core::{capture_transport,capture_image::CaptureImageBuilder,
            frame_budget::FrameLimits,worker_wire::WorkerReply};
        let (_,broker,image)=funded();
        let old=broker.state.lock().unwrap().entries[&image].worker.clone().unwrap();
        let size=FrameSize::new(32,18);
        let reply=WorkerReply::Reference {generation:7,regions:true,limits:FrameLimits::LEGACY,family_layout_revision:None,
            reply:Box::new(WorkerReply::Raw(CaptureImageBuilder::new(size,FrameLimits::LEGACY,&old,CaptureOwner::SourcePixels).unwrap().freeze()))};
        let prepared=capture_transport::prepare_owned(&reply,image,&old).unwrap().unwrap();
        let mut next=CapturePlan::new(old.source().clone(),CaptureGeneration::new(2));
        next.add_bgra(CaptureOwner::SourcePixels,size,2,CapturePhases::ALL).unwrap();
        let output=HostOutputPlan::new(HostContract::Legacy,size,HostCodecPlan::Tiles).unwrap();
        output.for_worker().append_to(&mut next,HostInputPixels::BorrowedSource).unwrap();
        let next=broker.approve(next,None).unwrap();
        let mut wire=Vec::new();
        capture_transport::send_prepared(&mut wire,&prepared).unwrap();
        let mut wire=std::io::Cursor::new(wire);
        let (_,length)=capture_transport::read_header(&mut wire).unwrap().unwrap();
        let (_,parent,payload)=capture_transport::read_owned(&mut wire,length,None,|id|broker.grant(id)).unwrap();
        let decoded=capture_transport::decode_owned(&payload,&parent,None).unwrap();
        assert_eq!(decoded,reply);
        drop(prepared);drop(reply);
        broker.retired(image).unwrap();
        assert!(!parent.is_retired().unwrap());
        assert!(parent.claim(CaptureOwner::Transport,64).is_ok(),"received Parent results keep their downstream plan");
        assert!(matches!(&decoded,WorkerReply::Reference {generation:7,..}));
        let WorkerReply::Reference {reply:decoded_image,..}=&decoded else {panic!("missing received reference");};
        let WorkerReply::Raw(decoded_image)=decoded_image.as_ref() else {panic!("missing received RAW image");};
        use crate::screen::core::host_backend::ScreenEncoder;
        let mut encoder=crate::screen::core::host_codec_adapters::AdmittedTilesEncoder::new(
            size,FrameLimits::LEGACY,decoded_image.grant()).unwrap();
        let request=crate::screen::core::host_stage::StageRequest {frame_contract:None,visible_rects:None,
            frame_id:2,keyframe:true,target_bps:1,size,choice:crate::screen::core::negotiate::CodecChoice {
                codec:crate::screen::core::negotiate::CodecKind::Tiles,profile:None,encoder_hardware:false,decoder_hardware:false,
                max_size:size,max_pixels:size.pixels(),max_macroblocks_per_second:None},encoder_epoch:2,fps:1};
        assert!(!encoder.encode_captured(decoded_image,&request).unwrap().records.is_empty(),
            "a received RAW snapshot still encodes after the Worker retired its credit");
        assert_eq!(broker.current().unwrap().generation().value(),next.generation);
        assert!(broker.grant(next.id).unwrap().claim(CaptureOwner::Transport,64).is_ok());
    }
}
