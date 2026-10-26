//! Private capture-control metadata. Descriptors are accepted only on the
//! existing verified Parent/worker pipes, never from a peer protocol.
use std::io;
use serde::{Deserialize,Serialize};
use super::capture_admission::{CaptureGeneration,CaptureOwner,CapturePlan,SourceKey};
use super::capture_plan::{CapturePhaseCase,CapturePhases,OWNER_COUNT};
use super::capture_source_contract::{HostOutputPlan,PreparedSourceInfo};
use super::host_codec_adapters::{HostCodecPlan,HostContract};
use super::frame_budget::{FrameContract,FrameLimits};
use super::pixel_convert::FrameSize;
use super::session_state::SessionState;
use super::window_list::SourceId;
use super::wire::invalid;

pub(crate) const CONTROL: u8=40;
pub(crate) const CONTROL_REPLY: u8=41;
pub(crate) const GRANT_REQUEST: u8=42;
pub(crate) const GRANT_REPLY: u8=43;
pub(crate) const RETIRED: u8=44;
pub(crate) const OWNED_REFERENCE: u8=45;
pub(crate) const REDUCED: u8=46;

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
pub(crate) struct OutputWire {pub contract:Option<FrameContract>,pub size:FrameSize,pub codec:u8,pub mf_buffer:u64,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub local_limits:Option<FrameLimits>,
}
impl OutputWire {
    pub(crate) fn from_plan(plan:&HostOutputPlan)->Self {
        let (codec,mf_buffer)=match plan.codec(){HostCodecPlan::Tiles=>(0,0),HostCodecPlan::OpenH264=>(1,0),
            HostCodecPlan::MediaFoundation{output_buffer_bytes}=>(2,output_buffer_bytes)};
        Self{contract:plan.contract().frame_contract(),size:plan.size(),codec,mf_buffer,local_limits:plan.legacy_limits()}
    }
    pub(crate) fn plan(&self)->io::Result<HostOutputPlan> {
        let codec=match self.codec {0 if self.mf_buffer==0=>HostCodecPlan::Tiles,1 if self.mf_buffer==0=>HostCodecPlan::OpenH264,
            2=>HostCodecPlan::MediaFoundation{output_buffer_bytes:self.mf_buffer},_=>return Err(invalid("unknown worker codec allocation plan"))};
        let plan=HostOutputPlan::new(self.contract.map_or(HostContract::Legacy,HostContract::Versioned),self.size,codec)?;
        match self.local_limits {Some(limits)=>plan.with_legacy_limits(limits),None=>Ok(plan)}
    }
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
pub(crate) struct PreparedInfoWire {pub source:String,pub size:Option<FrameSize>,pub limits:FrameLimits,
    pub regions:bool,pub state:SessionState,pub generation:u64}
impl PreparedInfoWire {
    pub(crate) fn new(source:&SourceKey,info:PreparedSourceInfo)->Self {
        Self{source:source.source().into(),size:info.native_size,limits:info.capture_limits,
            regions:info.supports_regions,state:info.state,generation:info.generation.value()}
    }
    pub(crate) fn info(&self)->io::Result<PreparedSourceInfo> {
        self.limits.validate_capture()?;
        if let Some(size)=self.size {super::capture_plan::bgra_bytes(size)?;}
        Ok(PreparedSourceInfo{native_size:self.size,capture_limits:self.limits,supports_regions:self.regions,
            state:self.state,generation:CaptureGeneration::new(self.generation)})
    }
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
pub(crate) struct RequirementWire {source:String,generation:u64,owner:u8,bytes:u64,count:u64,phases:u16}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
pub(crate) struct PlanWire {pub source:String,pub generation:u64,pub bound:bool,pub contract:Option<FrameContract>,
    requirements:Vec<RequirementWire>,cases:Vec<CapturePhaseCase>}
impl PlanWire {
    pub(crate) fn new(plan:&CapturePlan)->Self {
        Self{source:plan.source().source().into(),generation:plan.generation().value(),
            bound:plan.contract_binding().is_some(),contract:plan.contract_binding().flatten(),
            requirements:plan.requirements().iter().map(|item|RequirementWire {source:item.source().source().into(),
                generation:item.generation().value(),owner:item.owner() as u8,bytes:item.bytes_each(),count:item.count(),phases:item.phases().bits()}).collect(),
            cases:plan.concurrency_cases().to_vec()}
    }
    pub(crate) fn plan(&self,session:&str)->io::Result<CapturePlan> {
        let mut plan=CapturePlan::new(SourceKey::new(session,&self.source)?,CaptureGeneration::new(self.generation));
        if self.bound {plan.bind_contract(self.contract)?;}
        else if self.contract.is_some() {return Err(invalid("unbound worker plan contains a contract"));}
        for item in &self.requirements {
            let mut member=CapturePlan::new(SourceKey::new(session,&item.source)?,CaptureGeneration::new(item.generation));
            member.add_bytes(owner(item.owner)?,item.bytes,item.count,CapturePhases::from_bits(item.phases)?)?;
            plan.add_member(member)?;
        }
        plan.restore_verified_cases(self.cases.clone())?;
        Ok(plan)
    }
}
pub(crate) fn owner(value:u8)->io::Result<CaptureOwner> {
    use CaptureOwner::*;
    [NativeSurface,Readback,SourcePixels,CompositionPixels,ScaledPixels,FilterScratch,Metadata,Transport,CodecInput,CodecOutput]
        .get(value as usize).copied().ok_or_else(||invalid("unknown worker capture owner"))
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(tag="operation",content="value",rename_all="snake_case")]
pub(crate) enum CaptureRequest {
    Prepare{owner:String,session:String,source:SourceId,wanted:Option<FrameSize>,control:bool,working:u64,
        family:Option<super::worker_family::OpenFamily>},
    Plan(OutputWire),
    Activate{credit:CreditWire},
    Activity(bool),
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(tag="result",content="value",rename_all="snake_case")]
pub(crate) enum CaptureReply {
    Prepared(PreparedInfoWire),
    Plan{info:PreparedInfoWire,plan:Option<PlanWire>},
    Activated,
    Activity(bool),
}

#[path = "worker_capture_resource_wire.rs"]
mod resources;
pub(crate) use resources::{CreditWire, ImportNeedWire, GrantRequest, GrantValue, GrantReply, Reduction};
