//! C9 ownership fixtures, consumed by the one remote batch suite.
use std::{sync::{Arc,PoisonError},time::Duration};
use crate::screen::{core::{
    capture_admission::{CaptureGeneration,CaptureGrant,SessionAdmission,SourceKey},
    frame_budget::{FrameContract,FrameLimits,MAX_WORKING_BYTES},
    frame_memory::{CapacityCeiling,MemoryPool},
    host_backend::{HostOutputPlan,ScreenHostBackend},
    host_codec_adapters::{HostCodecPlan,HostContract},pixel_convert::FrameSize,window_list::SourceId,
},os::{bridge_channels::{self as channels,HostStart,Phase,FLAG_STILL},
    bridge_backend::BridgeBackend,android_capture_admission as admission}};
fn output()->HostOutputPlan {
    HostOutputPlan::new(HostContract::Legacy,FrameSize::new(64,32),HostCodecPlan::Tiles).unwrap()
}
fn session()->(Arc<channels::Channel>,Arc<SessionAdmission>,CaptureGrant) {
    let session="android-c9";
    let pool=Arc::new(MemoryPool::default());
    let admission=pool.admit_session(None,CapacityCeiling::bounded(MAX_WORKING_BYTES),MAX_WORKING_BYTES,session).unwrap();
    admission.bind_contract(None).unwrap();
    let channel=channels::open(HostStart {source:"monitor:0".into(),still:false,viewer_device_id:String::new(),
        viewer_name:String::new(),max_width:64,max_height:32});
    channel.capture.attach(admission.clone(),SourceKey::new(session,"monitor:0").unwrap()).unwrap();
    let plan=channel.capture.plan(FrameSize::new(64,32),admission::PROJECTION_TILES,&output()).unwrap();
    let grant=admission.authority().approve(plan).unwrap();channel.capture.activate(grant.clone()).unwrap();
    channels::ready(channel.id,Phase::Ready {width:64,height:32,encoder:"tiles".into(),hardware:false,projection:true}).unwrap();
    (channel,admission,grant)
}
fn still(value:u8)->Vec<u8> {
    let mut data=Vec::new();data.extend_from_slice(&64u32.to_le_bytes());data.extend_from_slice(&32u32.to_le_bytes());
    data.resize(8+64*32*4,value);data
}
#[test]
fn batches_capture_task_android_pending_metadata_cannot_allocate() {
    let _lock=channels::TEST_LOCK.lock().unwrap_or_else(PoisonError::into_inner);channels::reset_for_tests();
    channels::set_prereqs(|p|p.registered=true);
    let pool=Arc::new(MemoryPool::default());
    let a=pool.admit_session(None,CapacityCeiling::bounded(MAX_WORKING_BYTES),MAX_WORKING_BYTES,"pending").unwrap();
    a.bind_contract(None).unwrap();
    let prepared=BridgeBackend.prepare_session_source("owner","pending",&SourceId::Monitor("0".into()),None,false,a).unwrap();
    assert!(prepared.description().native_size.is_none());assert!(prepared.capture_plan(&output()).unwrap().is_none());
    let channel=channels::channels().pop().unwrap();
    assert!(channel.begin_capture(FrameSize::new(64,32),admission::PROJECTION_TILES).is_err());
    assert!(channel.capture.grant(1).is_err());
    assert_eq!(BridgeBackend.open_source(&SourceId::Monitor("0".into()),None).err().unwrap().reason,"capture_grant_required");
}
#[test]
fn batches_capture_task_android_three_rgba_slots_are_preallocated_credits() {
    let _lock=channels::TEST_LOCK.lock().unwrap_or_else(PoisonError::into_inner);channels::reset_for_tests();
    let (channel,_a,grant)=session();let n=64*32*4;
    let first=super::claim(&grant,2,n,true).unwrap();let second=super::claim(&grant,2,n,true).unwrap();
    let third=super::claim(&grant,2,n,true).unwrap();
    assert!(super::claim(&grant,2,n,true).is_err());assert_eq!(grant.live_bytes().unwrap(),3*n);
    assert_eq!(super::address(first).unwrap().1,n as usize);
    channels::close(channel.id,"viewer_closed");assert_eq!(grant.live_bytes().unwrap(),3*n);
    super::release(first);super::release(second);super::release(third);assert_eq!(grant.live_bytes().unwrap(),0);
}
#[test]
fn batches_capture_task_android_queue_evicts_before_replacement_copy() {
    let _lock=channels::TEST_LOCK.lock().unwrap_or_else(PoisonError::into_inner);channels::reset_for_tests();
    let (channel,_a,grant)=session();let generation=grant.generation().value();let first=still(1);let j=first.len() as u64;
    let producer=super::claim(&grant,7,j,true).unwrap();
    assert_eq!(channels::push_admitted(channel.id,generation,&first,FLAG_STILL),0);
    let consumer=channel.next_picture(Duration::ZERO,|_|true).unwrap().unwrap();
    assert_eq!(channels::push_admitted(channel.id,generation,&still(2),FLAG_STILL),0);
    assert_eq!(channels::push_admitted(channel.id,generation,&still(3),FLAG_STILL),0);
    assert_eq!(grant.live_bytes().unwrap(),3*j);
    let queued=channel.next_picture(Duration::ZERO,|_|true).unwrap().unwrap();
    assert_eq!(queued.data.as_slice()[8],3);drop(queued);
    channels::close(channel.id,"viewer_closed");assert_eq!(grant.live_bytes().unwrap(),2*j);
    drop(consumer);super::release(producer);assert_eq!(grant.live_bytes().unwrap(),0);
}
#[test]
fn batches_capture_task_android_resize_and_close_keep_old_callback_owner() {
    let _lock=channels::TEST_LOCK.lock().unwrap_or_else(PoisonError::into_inner);channels::reset_for_tests();
    let (channel,_a,grant)=session();let old=channel.begin_capture(FrameSize::new(64,32),admission::PROJECTION_TILES).unwrap();
    let reader=super::claim(&grant,0,64*32*4*3,false).unwrap();let pixels=super::claim(&grant,2,64*32*4,true).unwrap();
    let newer=channel.begin_capture(FrameSize::new(128,64),admission::PROJECTION_TILES).unwrap();
    assert!(newer>old);assert!(channel.capture.grant(old).is_err());
    assert_eq!(channels::push_admitted(channel.id,old,&still(1),FLAG_STILL),channels::PUSH_INVALID);
    channels::close(channel.id,"viewer_closed");assert!(super::address(pixels).is_some());
    assert_eq!(grant.live_bytes().unwrap(),64*32*4*4);
    super::release(pixels);super::release(reader);assert_eq!(grant.live_bytes().unwrap(),0);
}
#[test]
fn batches_capture_task_android_small_heap_is_not_raised_for_native_buffers() {
    let bytes=16*1024*1024;let pool=Arc::new(MemoryPool::default());
    let a=pool.admit_session(None,CapacityCeiling::bounded(bytes),bytes,"small-heap").unwrap();
    let contract=FrameContract {version:2,limits:FrameLimits::RESOURCE.with_working_budget(bytes).unwrap(),hybrid_regions:false};
    a.bind_contract(Some(contract)).unwrap();
    let output=HostOutputPlan::new(HostContract::Versioned(contract),FrameSize::new(64,32),HostCodecPlan::OpenH264).unwrap();
    let plan=admission::plan(&SourceKey::new("small-heap","monitor:0").unwrap(),CaptureGeneration::new(1),
        FrameSize::new(64,32),admission::PROJECTION_HYBRID,&output).unwrap();
    assert!(a.authority().approve(plan).is_err());assert_eq!(a.working_bytes(),bytes);
    assert!(pool.admit_session(None,CapacityCeiling::bounded(MAX_WORKING_BYTES),1024*1024,"second").is_err());
}

#[test]
fn batches_capture_task_android_accessibility_fits_source_only_output_before_begin() {
    let pool=Arc::new(MemoryPool::default());
    let a=pool.admit_session(None,CapacityCeiling::bounded(MAX_WORKING_BYTES),MAX_WORKING_BYTES,"wide-still").unwrap();
    a.bind_contract(None).unwrap();
    let capture=admission::CaptureSession::default();
    capture.attach(a.clone(),SourceKey::new("wide-still","monitor:0").unwrap()).unwrap();
    let native=FrameSize::new(8192,1024);
    let source=HostOutputPlan::for_source(HostContract::Legacy,native,FrameLimits::RESOURCE).unwrap();
    let grant=a.authority().approve(capture.plan(native,admission::ACCESSIBILITY,&source).unwrap()).unwrap();
    capture.activate(grant).unwrap();
    let generation=capture.begin(native,admission::ACCESSIBILITY).unwrap();
    let grant=capture.grant(generation).unwrap();
    let native_bytes=u64::from(native.width)*u64::from(native.height)*4;
    assert_eq!(grant.plan().owner_limits()[0],native_bytes);
    assert!(grant.plan().owner_limits()[4]>=FrameLimits::LEGACY.check(FrameLimits::LEGACY.fit(native)).unwrap() as u64);
}

#[test]
fn batches_capture_task_android_synthetic_source_only_adds_and_reuses_output_grant() {
    let pool=Arc::new(MemoryPool::default());
    let a=pool.admit_session(None,CapacityCeiling::bounded(MAX_WORKING_BYTES),MAX_WORKING_BYTES,"synthetic-tiles").unwrap();
    a.bind_contract(None).unwrap();
    let capture=admission::CaptureSession::default();
    capture.attach(a.clone(),SourceKey::new("synthetic-tiles","synthetic").unwrap()).unwrap();
    let size=FrameSize::new(128,72);
    let source=HostOutputPlan::for_source(HostContract::Legacy,size,FrameLimits::RESOURCE).unwrap();
    let initial=a.authority().approve(capture.plan(size,admission::SYNTHETIC_PIXELS,&source).unwrap()).unwrap();
    let initial_generation=initial.generation();capture.activate(initial).unwrap();
    let output=capture.synthetic_snapshot(size).unwrap();
    assert!(output.generation().value()>initial_generation.value());
    assert!(output.plan().owner_limits()[8]>=128*72*4);
    assert!(output.plan().owner_limits()[9]>0);
    assert_eq!(capture.synthetic_snapshot(size).unwrap().generation(),output.generation());
}

#[test]
fn batches_capture_task_android_refused_resize_preserves_previous_generation_and_owner() {
    let _lock=channels::TEST_LOCK.lock().unwrap_or_else(PoisonError::into_inner);channels::reset_for_tests();
    let (channel,_a,grant)=session();
    let generation=channel.begin_capture(FrameSize::new(64,32),admission::PROJECTION_TILES).unwrap();
    let owner=super::claim(&grant,2,64*32*4,true).unwrap();
    // Sixteen-megapixel hybrid capture exceeds this session before a new slot
    // exists. A refusal must not revoke the admitted callback's old generation.
    assert!(channel.begin_capture(FrameSize::new(4096,4096),admission::PROJECTION_HYBRID).is_err());
    assert_eq!(channel.capture.grant(generation).unwrap().generation(),grant.generation());
    assert_eq!(super::address(owner).unwrap().1,64*32*4);
    super::release(owner);channels::close(channel.id,"viewer_closed");
}
