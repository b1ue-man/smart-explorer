//! Android bridge compatibility through the mandatory prepare/activate boundary.
use std::sync::{Arc, MutexGuard, PoisonError};
use std::time::Duration;
use super::bridge_backend::{BridgeBackend, BridgeIndicator, HOST_APP_NOT_RUNNING};
use super::bridge_channels::{self as channels, Phase, FLAG_KEYFRAME, FLAG_STILL};
use super::bridge_source::{decode_still, epoch_flags};
use crate::screen::core::{
    capture_admission::{CaptureGrant,CaptureOwner,SessionAdmission},
    frame_budget::MAX_WORKING_BYTES,
    frame_memory::{CapacityCeiling,MemoryPool},
    host_backend::{HostOutputPlan,PreparedSource,ScreenHostBackend,ScreenSource,SessionIndicator},
    host_codec_adapters::{HostCodecPlan,HostContract},
    host_stage::{StageFrame,StageRequest},
    input_model::InputEvent,
    negotiate::{CodecChoice,CodecKind,H264Profile},
    pixel_convert::FrameSize,
    registry::ScreenRegistry,
    session_state::SessionState,
    window_list::SourceId,
    wire_control::ControlBlock,
    wire_hello::ScreenErrorCode,
};
fn fresh()->MutexGuard<'static,()> {
    let guard=channels::TEST_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    channels::reset_for_tests();guard
}
fn register_app(accessibility:&str) {
    channels::set_prereqs(|p| {p.registered=true;p.accessibility=accessibility.into();p.display=Some((1080,2400));});
}
fn prepare()->(Box<dyn PreparedSource>,Arc<SessionAdmission>) {
    let pool=Arc::new(MemoryPool::default());
    let admission=pool.admit_session(None,CapacityCeiling::bounded(MAX_WORKING_BYTES),MAX_WORKING_BYTES,"android-test").unwrap();
    admission.bind_contract(None).unwrap();
    let source=BridgeBackend.prepare_session_source("owner","android-test",&SourceId::Monitor("0".into()),None,false,admission.clone()).unwrap();
    (source,admission)
}
fn request(codec:CodecKind,epoch:u64,keyframe:bool)->StageRequest {
    StageRequest {frame_contract:None,visible_rects:None,frame_id:epoch*10,keyframe,target_bps:4_000_000,
        size:FrameSize::new(540,1200),choice:CodecChoice {max_macroblocks_per_second:None,codec,
            profile:(codec==CodecKind::H264).then_some(H264Profile::High),encoder_hardware:true,decoder_hardware:true,
            max_size:FrameSize::MAX,max_pixels:crate::screen::core::pixel_convert::MAX_FRAME_PIXELS},encoder_epoch:epoch,fps:30}
}
fn ready(id:u64,projection:bool) {
    channels::ready(id,Phase::Ready {width:1080,height:2400,encoder:"c2.qti.avc.encoder".into(),hardware:true,projection}).unwrap();
}
fn activate(source:Box<dyn PreparedSource>,admission:&SessionAdmission,codec:CodecKind)->Box<dyn ScreenSource> {
    let output=HostOutputPlan::new(HostContract::Legacy,FrameSize::new(540,1200),
        if codec==CodecKind::Tiles {HostCodecPlan::Tiles} else {HostCodecPlan::OpenH264}).unwrap();
    let grant=admission.authority().approve(source.capture_plan(&output).unwrap().unwrap()).unwrap();
    source.activate(grant).unwrap()
}
fn push(id:u64,grant:&CaptureGrant,data:&[u8],flags:u32)->i32 {
    channels::push_admitted(id,grant.generation().value(),data,flags)
}
fn drain(id:u64)->Vec<String> {
    std::iter::from_fn(||channels::control(id,Duration::from_millis(1))).take(16).collect()
}
#[test]
fn screen_task_android_stage_waits_for_consent_and_ends_on_refusal() {
    let _guard=fresh();let monitor=SourceId::Monitor("0".into());
    assert!(!BridgeIndicator::new(&Arc::new(ScreenRegistry::default())).available());
    assert_eq!(BridgeBackend.open_source(&monitor,None).err().unwrap().reason,HOST_APP_NOT_RUNNING);
    register_app("off");let (prepared,_admission)=prepare();
    assert_eq!(prepared.description().state,SessionState::WaitingForPortal);
    let output=HostOutputPlan::new(HostContract::Legacy,FrameSize::new(540,1200),HostCodecPlan::OpenH264).unwrap();
    assert!(prepared.capture_plan(&output).unwrap().is_none());
    let id=channels::channels()[0].id;
    channels::ended(id,"consent_denied").unwrap();
    assert!(prepared.description().native_size.is_none());
    drop(prepared);assert!(channels::channels().is_empty());
    assert!(drain(id).iter().any(|m|m.contains("\"stop\"")));
}
#[test]
fn screen_task_android_stage_configures_and_takes_key_frames() {
    let _guard=fresh();register_app("off");let (prepared,admission)=prepare();
    let id=channels::channels()[0].id;ready(id,true);
    let mut source=activate(prepared,&admission,CodecKind::H264);
    let grant=source.capture_grant().unwrap();let copy=grant.clone();
    let app=std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(push(id,&copy,b"old",epoch_flags(1)|FLAG_KEYFRAME),0);
        assert_eq!(push(id,&copy,b"delta",epoch_flags(2)),0);
        assert_eq!(push(id,&copy,b"\0\0\0\x01key",epoch_flags(2)|FLAG_KEYFRAME),0);
    });
    let stage=source.host_stage().unwrap();
    match stage.produce(&request(CodecKind::H264,2,true)).unwrap() {
        StageFrame::Encoded {records,keyframe,..}=> {assert!(keyframe);assert_eq!(records.as_records(),&[b"\0\0\0\x01key".to_vec()]);},
        other=>panic!("expected keyframe, got {other:?}"),
    }
    app.join().unwrap();let sent=drain(id);
    assert!(sent[0].contains("\"op\":\"configure\"") && sent[0].contains("\"width\":540"));
    assert!(matches!(stage.produce(&request(CodecKind::H264,2,false)).unwrap(),StageFrame::Unchanged {..}));
    assert!(drain(id).is_empty());
    assert_eq!(push(id,&grant,b"",FLAG_KEYFRAME),channels::PUSH_INVALID);
    assert_eq!(push(id+99,&grant,b"x",FLAG_KEYFRAME),channels::PUSH_UNKNOWN);
    channels::ended(id,"consent_denied").unwrap();
    assert_eq!(stage.produce(&request(CodecKind::H264,2,true)).err().unwrap().code,ScreenErrorCode::ApprovalDenied);
}
#[test]
fn screen_task_android_still_decodes_and_tiles() {
    let _guard=fresh();register_app("on");let (prepared,admission)=prepare();
    let id=channels::channels()[0].id;ready(id,false);
    let mut source=activate(prepared,&admission,CodecKind::Tiles);let grant=source.capture_grant().unwrap();
    let mut small=Vec::new();small.extend_from_slice(&4u32.to_le_bytes());small.extend_from_slice(&2u32.to_le_bytes());
    small.extend(std::iter::repeat_n([10u8,20,30,0],8).flatten());
    let image=decode_still(&small,&grant,CaptureOwner::SourcePixels).unwrap();
    assert_eq!(&image.image().data()[..4],&[30,20,10,255]);
    assert!(decode_still(&small[..12],&grant,CaptureOwner::SourcePixels).is_err());drop(image);
    let mut full=Vec::new();full.extend_from_slice(&1080u32.to_le_bytes());full.extend_from_slice(&2400u32.to_le_bytes());
    full.resize(8+1080*2400*4,0x80);assert_eq!(push(id,&grant,&full,FLAG_STILL),0);
    let stage=source.host_stage().unwrap();
    match stage.produce(&request(CodecKind::Tiles,3,true)).unwrap() {
        StageFrame::Encoded {records,encoder,..}=> {assert_eq!(encoder.name,"tiles");assert!(!records.is_empty());},
        other=>panic!("expected tiles, got {other:?}"),
    }
    assert!(drain(id).iter().any(|m|m.contains("\"op\":\"still\"")));
    assert_eq!(stage.produce(&request(CodecKind::H264,4,true)).unwrap(),StageFrame::Paused(SessionState::WaitingForPortal));
    assert!(drain(id).iter().any(|m|m.contains("\"op\":\"consent\"")));
}
#[test]
fn screen_task_android_input_needs_accessibility_and_stream() {
    let _guard=fresh();let events=[InputEvent::Text {text:"hallo".into()}];
    register_app("off");assert_eq!(BridgeBackend.inject(&events),Err(ControlBlock::ControlUnavailable));
    register_app("on");assert_eq!(BridgeBackend.inject(&events),Err(ControlBlock::ControlUnavailable));
    let (_prepared,_admission)=prepare();let id=channels::channels()[0].id;ready(id,true);
    assert!(BridgeBackend.input().touch);assert_eq!(BridgeBackend.inject(&events),Ok(()));
    let input=drain(channels::INPUT_CHANNEL);assert_eq!(input.len(),1);assert!(input[0].contains("hallo"));
    assert_eq!(BridgeBackend.limits(),["consent_each_session","no_key_events","still_max_3_per_second","android_system_selected_source"]);
}

#[test]
fn batches_capture_task_android_wide_native_still_scales_once_without_false_resize() {
    let _guard=fresh();register_app("on");let (prepared,admission)=prepare();
    let id=channels::channels()[0].id;
    let native=FrameSize::new(8192,8);let size=FrameSize::new(4096,4);
    channels::ready(id,Phase::Ready {width:native.width,height:native.height,
        encoder:"tiles".into(),hardware:false,projection:false}).unwrap();
    let output=HostOutputPlan::new(HostContract::Legacy,size,HostCodecPlan::Tiles).unwrap();
    let grant=admission.authority().approve(prepared.capture_plan(&output).unwrap().unwrap()).unwrap();
    let mut source=prepared.activate(grant.clone()).unwrap();
    let mut bytes=Vec::new();bytes.extend_from_slice(&native.width.to_le_bytes());
    bytes.extend_from_slice(&native.height.to_le_bytes());bytes.resize(8+8192*8*4,0x80);
    assert_eq!(push(id,&grant,&bytes,FLAG_STILL),0);
    let mut wanted=request(CodecKind::Tiles,1,true);wanted.size=size;
    match source.host_stage().unwrap().produce(&wanted).unwrap() {
        StageFrame::Encoded {source_size,records,..}=>{assert_eq!(source_size,native);assert!(!records.is_empty());},
        frame=>panic!("native geometry must not become a scaled resize event: {frame:?}"),
    }
}
