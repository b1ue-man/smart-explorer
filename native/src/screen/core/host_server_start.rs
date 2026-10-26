//! One W promise precedes metadata preparation. Only the final negotiated
//! binding can authorize a frame plan; unfinished native metadata stays pending.
use super::*;
use crate::screen::core::{capture_admission::SessionAdmission,capture_source_contract::PreparedSource,
    host_backend::ScreenSource,host_codec_adapters::{HostCodecPlan,HostContract,HostOutputPlan},pixel_convert::FrameSize};

pub(super) struct Ready {
    pub source:Box<dyn ScreenSource>,
    pub contract:Option<FrameContract>,
    pub choice:super::super::negotiate::CodecChoice,
    pub admission:Arc<SessionAdmission>,
}
pub(super) enum Failure {Capture(io::Error),Source(super::super::host_backend::SourceRefusal),Encoder,Cancelled}
struct PreparedOutput {
    result:Result<Box<dyn PreparedSource>,super::super::host_backend::SourceRefusal>,
    _admission:Arc<SessionAdmission>,
}
pub(super) async fn prepare(context:&HostContext,principal:&ProvenPrincipal,hello:&ScreenClientHello,
    source_id:&SourceId,connection:&Connection,handle:&SessionHandle)->Result<Ready,Failure> {
    let backend=context.backend.as_ref();
    let session=super::super::family_protocol::token().map_err(Failure::Capture)?;
    let facts=backend.memory_facts();
    let ceiling=backend.capacity_ceiling(facts);
    let offered=backend.session_memory_ceiling().min(hello.frame_capabilities
        .map_or(super::super::frame_budget::MAX_WORKING_BYTES,|caps|caps.frames.working_bytes));
    let admission=super::super::frame_memory::admit_session(facts,ceiling,offered,&session).map_err(Failure::Capture)?;
    let mut stop=handle.stop_signal();
    if stop.borrow().is_some() {return Err(Failure::Cancelled);}
    let prepared={
        let context=context.clone();let principal=principal.clone();let source=source_id.clone();
        let wanted=hello.display_size;let control=hello.mode==SessionMode::Control;let admission=admission.clone();
        tokio::task::spawn_blocking(move|| {
            let result=super::super::family_listing::prepare(&context,&principal,&source,wanted,control,admission.clone());
            PreparedOutput{result,_admission:admission}
        })
    };
    let prepared=tokio::select! {biased;
        _=connection.closed()=>return Err(Failure::Cancelled),
        _=stop.changed()=>return Err(Failure::Cancelled),
        result=prepared=>result.map_err(|error|Failure::Capture(io::Error::other(error)))?,
    };
    let prepared=prepared.result.map_err(Failure::Source)?;
    let info=prepared.description();info.capture_limits.validate_capture().map_err(Failure::Capture)?;
    let mut host=FrameCapabilities::admitted(info.capture_limits.max_size,admission.working_bytes()).map_err(Failure::Capture)?;
    host.frames=host.frames.for_capture(info.capture_limits);host.hybrid_regions=info.supports_regions;
    let mut contract=FrameContract::negotiate(Some(host),hello.frame_capabilities).map_err(Failure::Capture)?;
    let mut choices=negotiate_for_contract(&backend.encoders(),&hello.decoders,CodecPreference::Auto,contract);
    if choices.is_empty() && contract.is_some_and(|contract|contract.hybrid_regions)
        && hello.decoders.iter().any(|offer|offer.codec==super::super::negotiate::CodecKind::H264) {
        if let Some(contract)=contract.as_mut() {contract.hybrid_regions=false;}
        choices=negotiate_for_contract(&backend.encoders(),&hello.decoders,CodecPreference::Auto,contract);
    }
    let choice=choices.first().copied().ok_or(Failure::Encoder)?;
    admission.bind_contract(contract).map_err(Failure::Capture)?;
    let limits=match contract {Some(contract)=>contract.limits,None=>
        super::super::frame_budget::legacy_limits(admission.working_bytes(),None).map_err(Failure::Capture)?};
    // This is the encoder envelope, not a fabricated native source size.
    let size=limits.fit(limits.max_size);
    let codec=if choice.codec==super::super::negotiate::CodecKind::Tiles {HostCodecPlan::Tiles}
        else if backend.platform()==super::super::wire_hello::HostPlatform::Windows {
            HostCodecPlan::MediaFoundation{output_buffer_bytes:64*1024*1024}
        } else {HostCodecPlan::OpenH264};
    let output=HostOutputPlan::new(contract.map_or(HostContract::Legacy,HostContract::Versioned),size,codec).map_err(Failure::Capture)?;
    let output=if contract.is_none() {output.with_legacy_limits(limits).map_err(Failure::Capture)?} else {output};
    let allowed={let context=context.clone();let principal=principal.clone();let control=hello.mode==SessionMode::Control;
        Arc::new(move||super::super::family_listing::admitted(&context,&principal,control))};
    let source=super::super::host_admission::pending(prepared,admission.clone(),output,allowed);
    Ok(Ready{source,contract,choice,admission})
}
