//! Private transport and legacy interoperability cases for the remote suite.
use std::{io,sync::Arc,time::Duration};
use crate::screen::core::{
    frame_budget::{self,FrameCapabilities},frame_memory::{CapacityCeiling,MemoryPool},
    gop_stream::{GopWriter,PictureEvent},
    negotiate::CodecKind,pixel_convert::{BgraImage,FrameSize},tiles_codec::{TileEncoder,MAX_TILES_RECORD_BYTES},
    viewer_client,wire_binary::{GopHeader,StreamKind},
    wire_hello::{self,HelloReply,ScreenClientHello,ScreenHelloOk,ScreenServerHello},
};
const MIB:u64=1024*1024;
fn client()->ScreenClientHello {
    serde_json::from_value(serde_json::json!({"protocol_version":1,"relation_kind":"direct","relation_id":"fixture",
        "device_id":"viewer","device_name":"viewer","public_key":"key","fingerprint":"pin","node_id":"node",
        "client_nonce":"nonce","proof":"proof","mode":"preview","source":"synthetic",
        "decoders":[{"codec":"tiles"}],"display_size":{"width":320,"height":320},"max_fps":1})).unwrap()
}
fn legacy_accepted()->ScreenHelloOk {
    serde_json::from_value(serde_json::json!({"authorization":{"view":"allow","control":"allow"},
        "platform":"windows","approval":"not_required"})).unwrap()
}

#[test]
fn batches_task_b6_session_budget_android_policy_shares_previews_desktop_keeps_full_contract() {
    use crate::screen::core::frame_budget::PreviewAdmission;
    let server=ScreenServerHello::new("challenge".into(),"host".into(),"key".into(),"pin".into(),"node".into());
    let mut request=client();
    request.frame_capabilities=Some(FrameCapabilities::admitted(FrameSize::new(4096,4096),64*MIB).unwrap());
    let pool=Arc::new(MemoryPool::default());let ceiling=CapacityCeiling::bounded(64*MIB);
    let reserve=|bytes|pool.reserve_with_ceiling(None,ceiling,bytes);
    let (first,first_memory)=super::admit_hello(request.clone(),&server,PreviewAdmission::Geometric,reserve).unwrap();
    let (second,second_memory)=super::admit_hello(request.clone(),&server,PreviewAdmission::Geometric,reserve).unwrap();
    assert!(first_memory.unwrap().bytes()<32*MIB);assert!(second_memory.unwrap().bytes()<32*MIB);
    assert_eq!(first.frame_capabilities.unwrap().frames.max_size,FrameSize::new(320,320));
    assert_eq!(second.frame_capabilities.unwrap().frames.max_size,FrameSize::new(320,320));
    let desktop=Arc::new(MemoryPool::default());
    let (hello,memory)=super::admit_hello(request,&server,PreviewAdmission::FullContract,
        |bytes|desktop.reserve_with_ceiling(None,ceiling,bytes)).unwrap();
    assert_eq!(memory.unwrap().bytes(),64*MIB);
    assert!(hello.frame_capabilities.unwrap().frames.check(FrameSize::new(960,540)).is_ok(),
        "desktop promotes the same session to full view");
    let mut probe=client();probe.mode=crate::screen::core::wire_hello::SessionMode::Probe;
    let (hello,memory)=super::admit_hello(probe,&server,PreviewAdmission::Geometric,
        |_|panic!("a capability probe must not reserve image memory")).unwrap();
    assert!(memory.is_none());assert!(hello.frame_capabilities.is_none());
}
async fn endpoints()->(iroh::Endpoint,iroh::Endpoint,iroh::endpoint::Connection,iroh::endpoint::Connection) {
    async fn endpoint()->iroh::Endpoint {
        iroh::Endpoint::builder(iroh::endpoint::presets::Minimal).clear_address_lookup()
            .relay_mode(iroh::RelayMode::Disabled).clear_ip_transports().bind_addr("127.0.0.1:0").unwrap()
            .alpns(vec![b"b6-session-budget/1".to_vec()]).bind().await.unwrap()
    }
    let left=endpoint().await;let right=endpoint().await;
    let address=iroh::EndpointAddr::from_parts(right.id(),right.bound_sockets().into_iter().map(iroh::TransportAddr::Ip));
    let (outgoing,incoming)=tokio::join!(left.connect(address,b"b6-session-budget/1"),async {right.accept().await.unwrap().await});
    (left,right,outgoing.unwrap(),incoming.unwrap())
}

#[test]
fn batches_task_b6_session_budget_v184_hello_and_picture_work_below_512_mib() {
    let runtime=tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    runtime.block_on(async {tokio::time::timeout(Duration::from_secs(20),async {
        let (left,right,viewer,host)=endpoints().await;
        let served=tokio::spawn(async move {
            let (mut send,mut recv)=host.open_bi().await.unwrap();
            let mut server=ScreenServerHello::new("challenge".into(),"host".into(),"key".into(),"pin".into(),"node".into());
            server.frame_memory_class=None;server.family_version=None;server.icon_version=None;
            wire_hello::send_server_hello(&mut send,&server).await.unwrap();
            // The released host ignores the unknown optional resource offer.
            #[derive(serde::Deserialize)]
            struct LegacyClient {display_size:Option<FrameSize>,source:Option<crate::screen::core::window_list::SourceId>}
            let hello:LegacyClient=crate::screen::core::wire::recv_json(&mut recv,crate::screen::core::wire::CLIENT_HELLO).await.unwrap();
            assert_eq!(hello.display_size,Some(FrameSize::new(320,320)));
            assert_eq!(hello.source,Some(crate::screen::core::window_list::SourceId::Synthetic));
            wire_hello::send_hello_reply(&mut send,&HelloReply::Accepted(legacy_accepted())).await.unwrap();
            let size=FrameSize::new(320,320);let image=BgraImage::new(size).unwrap();
            let records=TileEncoder::new(size).unwrap().encode(&image.view(),true,MAX_TILES_RECORD_BYTES).unwrap();
            let header=GopHeader {gop_id:7,frame_contract:None,family_layout_revision:None,
                kind:StreamKind::Preview,codec:CodecKind::Tiles,profile:None,size};
            let mut gop=GopWriter::open(&host,header).await.unwrap();gop.write_frame(1,123,true,records).await.unwrap();gop.finish();
            host.closed().await;
        });
        let pool=Arc::new(MemoryPool::default());let request=client();
        let wanted=frame_budget::requested_bytes(&request,64*MIB).unwrap();
        let lease=pool.reserve_with_ceiling(None,CapacityCeiling::bounded(64*MIB),wanted).unwrap();
        let offered=FrameCapabilities::admitted(FrameSize::new(4096,4096),lease.bytes()).unwrap();
        let mut request=request;request.frame_capabilities=Some(frame_budget::preview_capabilities(offered,&request).unwrap());
        let mut session=viewer_client::handshake_admitted(viewer,|_|Ok((request,Some(Arc::new(lease)))),|_|{}).await.unwrap();
        assert!(session.hello.frame_contract.is_none());
        let local=session.local_limits.unwrap();assert!(local.working_bytes<64*MIB);
        assert!(local.check(FrameSize::new(320,320)).is_ok());
        let renderer=session.memory.clone();
        loop {if let PictureEvent::Record {header,record,..}=session.pictures.recv().await.unwrap() {
            assert!(header.frame_contract.is_none());assert_eq!(header.gop_id,7);
            assert_eq!(record.frame_id,1);assert!(!record.payload.is_empty());break;
        }}
        session.close();drop(session);served.await.unwrap();left.close().await;right.close().await;
        assert!(pool.reserve_with_ceiling(None,CapacityCeiling::bounded(64*MIB),64*MIB).unwrap().bytes()<64*MIB,
            "the retained consumer still owns its actual lease");
        drop(renderer);
    }).await.unwrap();});
}

fn capture_restart()->(crate::screen::os::android_capture_admission::CaptureSession,
    Arc<crate::screen::core::capture_admission::SessionAdmission>,crate::screen::core::capture_admission::CaptureGrant,u64) {
    use crate::screen::{os::android_capture_admission as android,core::{
        capture_admission::{CaptureGeneration,SourceKey},host_codec_adapters::{HostCodecPlan,HostContract,HostOutputPlan},
    }};
    let size=FrameSize::new(64,32);let working=8*MIB;
    let caps=FrameCapabilities::admitted(size,working).unwrap();
    let contract=frame_budget::FrameContract::negotiate(Some(caps),Some(caps)).unwrap().unwrap();
    let output=HostOutputPlan::new(HostContract::Versioned(contract),size,HostCodecPlan::OpenH264).unwrap();
    let source=SourceKey::new("android-restart","synthetic").unwrap();
    let first=android::plan(&source,CaptureGeneration::new(1),size,android::SYNTHETIC_SURFACE,&output).unwrap();
    let peak=first.peak_bytes();let capacity=peak+MIB;assert!(2*peak>capacity);
    let pool=Arc::new(MemoryPool::default());
    let admission=pool.admit_session(None,CapacityCeiling::bounded(capacity),working,"android-restart").unwrap();
    admission.bind_contract(Some(contract)).unwrap();
    let capture=android::CaptureSession::default();capture.attach(admission.clone(),source).unwrap();
    let first=capture.plan(size,android::SYNTHETIC_SURFACE,&output).unwrap();
    let grant=admission.authority().approve(first).unwrap();capture.activate(grant.clone()).unwrap();
    assert_eq!(capture.begin(size,android::SYNTHETIC_SURFACE).unwrap(),1);
    (capture,admission,grant,peak)
}

#[test]
fn batches_task_b6_session_budget_android_resume_reuses_capacity_after_actual_owner_end() {
    use crate::screen::{os::android_capture_admission as android,core::capture_admission::CaptureOwner};
    let (capture,admission,old,peak)=capture_restart();
    let encoder=old.claim(CaptureOwner::CodecOutput,android::ACCESS_UNIT).unwrap();
    let surface=old.claim(CaptureOwner::NativeSurface,64*32*4).unwrap();
    // The existing Java pause path acknowledges only after these real owners end.
    drop(encoder);drop(surface);assert_eq!(old.live_bytes().unwrap(),0);
    let next=capture.begin(FrameSize::new(64,32),android::SYNTHETIC_SURFACE).unwrap();
    assert_eq!(next,2);assert!(old.is_retired().unwrap());assert!(capture.grant(1).is_err());
    assert_eq!(capture.grant(next).unwrap().plan().peak_bytes(),peak);
    assert_eq!(admission.additional_bytes().unwrap(),peak-admission.working_bytes());
}

#[test]
fn batches_task_b6_session_budget_android_resume_keeps_live_owner_and_retries_same_generation() {
    use crate::screen::{os::android_capture_admission as android,core::capture_admission::CaptureOwner};
    let (capture,admission,old,peak)=capture_restart();
    let retiring=old.claim(CaptureOwner::CodecOutput,android::ACCESS_UNIT).unwrap();
    let error=capture.begin(FrameSize::new(64,32),android::SYNTHETIC_SURFACE).unwrap_err();
    assert_eq!(error.kind(),io::ErrorKind::WouldBlock);
    assert!(old.is_retired().unwrap());assert_eq!(old.live_bytes().unwrap(),android::ACCESS_UNIT);
    assert!(old.claim(CaptureOwner::CodecOutput,1).is_err());
    let acknowledgement=capture.activity_ack(&[0;9]).unwrap();
    assert_eq!(acknowledgement.len(),9);assert_eq!(old.live_bytes().unwrap(),android::ACCESS_UNIT);
    assert_eq!(admission.additional_bytes().unwrap(),android::ACCESS_UNIT+9-admission.working_bytes());
    assert!(capture.activity_ack(&[0;10]).is_err());drop(acknowledgement);
    assert_eq!(admission.additional_bytes().unwrap(),android::ACCESS_UNIT-admission.working_bytes());
    drop(retiring);
    assert_eq!(capture.begin(FrameSize::new(64,32),android::SYNTHETIC_SURFACE).unwrap(),2);
    assert_eq!(admission.additional_bytes().unwrap(),peak-admission.working_bytes());
}

#[test]
fn batches_task_b6_session_budget_android_snapshot_replaces_only_retired_or_native_authority() {
    use crate::screen::{os::android_capture_admission as android,core::{
        capture_admission::{CaptureGeneration,CaptureOwner,SourceKey},
        host_codec_adapters::{HostCodecPlan,HostContract,HostOutputPlan},
    }};
    let (active,_,active_surface,_)=capture_restart();
    let parallel=active.synthetic_snapshot(FrameSize::new(64,32)).unwrap();
    assert!(!active_surface.is_retired().unwrap());
    assert_eq!(active.current().unwrap().book_identity(),active_surface.book_identity());
    assert_ne!(parallel.book_identity(),active_surface.book_identity());drop(parallel);drop(active);
    let (surface,_,old,_)=capture_restart();
    let live=old.claim(CaptureOwner::CodecOutput,android::ACCESS_UNIT).unwrap();
    assert_eq!(surface.begin(FrameSize::new(64,32),android::SYNTHETIC_SURFACE).unwrap_err().kind(),io::ErrorKind::WouldBlock);
    let snapshot=surface.synthetic_snapshot(FrameSize::new(64,32)).unwrap();
    assert!(!snapshot.is_retired().unwrap());assert_eq!(snapshot.generation().value(),2);
    let pixels=snapshot.claim(CaptureOwner::SourcePixels,64*32*4).unwrap();
    assert_eq!(old.live_bytes().unwrap(),android::ACCESS_UNIT);drop(pixels);drop(live);
    assert_eq!(surface.current().unwrap().generation(),snapshot.generation());

    let output_size=FrameSize::new(64,32);let working=MIB;
    let caps=FrameCapabilities::admitted(output_size,working).unwrap();
    let contract=frame_budget::FrameContract::negotiate(Some(caps),Some(caps)).unwrap().unwrap();
    let output=HostOutputPlan::new(HostContract::Versioned(contract),output_size,HostCodecPlan::Tiles).unwrap();
    let source=SourceKey::new("native-snapshot","synthetic").unwrap();
    let first_size=FrameSize::new(1024,512);let next_size=FrameSize::new(1024,768);
    let first=android::plan(&source,CaptureGeneration::new(1),first_size,android::SYNTHETIC_PIXELS,&output).unwrap();
    let next=android::plan(&source,CaptureGeneration::new(2),next_size,android::SYNTHETIC_PIXELS,&output).unwrap();
    let capacity=next.peak_bytes()+4096;assert!(first.peak_bytes()+next.peak_bytes()>capacity);
    let pool=Arc::new(MemoryPool::default());
    let admission=pool.admit_session(None,CapacityCeiling::bounded(capacity),working,"native-snapshot").unwrap();
    admission.bind_contract(Some(contract)).unwrap();
    let native=android::CaptureSession::default();native.attach(admission.clone(),source).unwrap();
    let first=native.plan(first_size,android::SYNTHETIC_PIXELS,&output).unwrap();
    let old=admission.authority().approve(first).unwrap();native.activate(old.clone()).unwrap();
    let held=old.claim(CaptureOwner::SourcePixels,16).unwrap();
    let next=native.synthetic_snapshot(next_size).unwrap();
    assert_eq!(next.generation().value(),2);assert!(old.is_retired().unwrap());
    assert_eq!(old.live_bytes().unwrap(),16);
    assert_eq!(admission.additional_bytes().unwrap(),next.plan().peak_bytes()+16-working);
    let pixels=next.claim(CaptureOwner::SourcePixels,next_size.pixels()*4).unwrap();
    assert_eq!(native.synthetic_snapshot(next_size).unwrap().generation(),next.generation());
    drop(pixels);drop(held);assert_eq!(old.live_bytes().unwrap(),0);
}

#[test]
fn batches_task_b6_session_budget_android_epoch_drops_old_queue_but_keeps_external_owner() {
    use crate::screen::{os::{bridge_channels as channels,android_capture_admission as android},core::{
        capture_admission::{CaptureOwner,SourceKey},host_codec_adapters::{HostCodecPlan,HostContract,HostOutputPlan},
    }};
    let _lock=channels::TEST_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    channels::reset_for_tests();
    let pool=Arc::new(MemoryPool::default());
    let admission=pool.admit_session(None,CapacityCeiling::bounded(512*MIB),512*MIB,"epoch").unwrap();
    admission.bind_contract(None).unwrap();
    let channel=channels::open(channels::HostStart {source:"synthetic".into(),still:false,
        viewer_device_id:String::new(),viewer_name:String::new(),max_width:64,max_height:32});
    channel.capture.attach(admission.clone(),SourceKey::new("epoch","synthetic").unwrap()).unwrap();
    let size=FrameSize::new(64,32);let n=size.pixels()*4;
    let output=HostOutputPlan::new(HostContract::Legacy,size,HostCodecPlan::Tiles).unwrap();
    let grant=admission.authority().approve(channel.capture.plan(size,android::PROJECTION_TILES,&output).unwrap()).unwrap();
    channel.capture.activate(grant.clone()).unwrap();
    let held=grant.claim(CaptureOwner::SourcePixels,n).unwrap();
    let mut picture=Vec::new();picture.extend_from_slice(&64u32.to_le_bytes());picture.extend_from_slice(&32u32.to_le_bytes());
    picture.resize(n as usize+8,0);
    assert_eq!(channels::push_admitted(channel.id,1,&picture,channels::FLAG_STILL|(1<<channels::EPOCH_SHIFT)),channels::PUSH_OK);
    assert_eq!(grant.live_bytes().unwrap(),2*n+8);
    channel.send(serde_json::json!({"op":"configure","epoch":2}));
    assert_eq!(grant.live_bytes().unwrap(),n);
    assert_eq!(channels::push_admitted(channel.id,1,&picture,channels::FLAG_STILL|(1<<channels::EPOCH_SHIFT)),channels::PUSH_OK);
    assert_eq!(grant.live_bytes().unwrap(),n,"old-epoch payload acquired no new queue owner");
    channels::close(channel.id,"viewer_closed");assert_eq!(grant.live_bytes().unwrap(),n);
    drop(held);assert_eq!(grant.live_bytes().unwrap(),0);
}
