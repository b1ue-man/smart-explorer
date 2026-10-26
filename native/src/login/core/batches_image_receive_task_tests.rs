//! L2 productive wire/codec fixtures, for the one remote task suite only.
use std::{io::Cursor, sync::{Arc,Mutex,Condvar,atomic::{AtomicBool,AtomicUsize,Ordering}}};
use super::{gateway_wire as wire,gateway_types::GatewayReply,image_receive::{self,ImageReceiver},image_send,wire::Reply};
use crate::screen::core::{capture_admission::{SourceKey,CaptureGeneration},capture_image::CaptureImageBuilder,
    frame_budget::FrameLimits,frame_memory::{MemoryPool,CapacityCeiling},image_admission::{ImageAdmission,ImageOwnerLimits,ImageRole,LocalImagePolicy},pixel_convert::FrameSize};
fn images(limits: ImageOwnerLimits,bytes: u64)->Arc<ImageAdmission> {
    ImageAdmission::in_test_pool(Arc::new(MemoryPool::default()),LocalImagePolicy {facts:None,ceiling:CapacityCeiling::bounded(bytes)},limits)
}
fn packet()->GatewayReply {
    let mut reply=Reply::state("pixels");reply.width=1;reply.height=1;reply.pixel_bytes=4;
    GatewayReply {request_sequence:1,reply,access:None}
}
fn receiver(admission: Arc<ImageAdmission>)->ImageReceiver {
    ImageReceiver {admission,source:SourceKey::new("fixture","login:network").unwrap(),cancelled:Arc::new(||false),lifetime:None}
}
fn png()->Vec<u8> {
    let mut bytes=Vec::new();
    {let mut encoder=png::Encoder::new(&mut bytes,1,1);encoder.set_color(png::ColorType::Rgb);encoder.set_depth(png::BitDepth::Eight);
    let mut writer=encoder.write_header().unwrap();writer.write_image_data(&[3,2,1]).unwrap();writer.finish().unwrap();}
    bytes
}
fn wire_bytes(png: &[u8])->Vec<u8> {
    let header=serde_json::to_vec(&serde_json::json!({"packet":packet(),"image_encoding":"png_rgb8","image_bytes":png.len()})).unwrap();
    let mut bytes=vec![1];bytes.extend_from_slice(&(header.len() as u32).to_be_bytes());bytes.extend_from_slice(&header);
    bytes.push(3);bytes.extend_from_slice(&(png.len() as u32).to_be_bytes());bytes.extend_from_slice(png);bytes
}
fn runtime()->tokio::runtime::Runtime {tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()}
struct End(Mutex<Option<tokio::sync::oneshot::Sender<()>>>);
impl Drop for End {fn drop(&mut self) {if let Some(done)=self.0.lock().unwrap().take(){let _=done.send(());}}}
struct ReleaseGate(Arc<(Mutex<bool>,Condvar)>);
impl Drop for ReleaseGate {fn drop(&mut self) {let mut released=self.0.0.lock().unwrap();*released=true;self.0.1.notify_all();}}
#[test]
fn batches_task_login_image_productive_png_keeps_sender_and_last_receiver_owner() {
    runtime().block_on(async {
        let sender=images(image_send::limits().unwrap(),512*1024*1024);
        let mut input=CaptureImageBuilder::new_local(FrameSize::new(1,1),FrameLimits::LEGACY,&sender,ImageRole::DecodedInput,
            SourceKey::new("sender","login:broker").unwrap(),CaptureGeneration::new(1)).unwrap();
        input.pixels_mut().copy_from_slice(&[1,2,3,255]);
        let encoded=wire::prepare_reply(packet(),Some(input.freeze()),sender.clone()).await.unwrap();
        assert_eq!(sender.live_count(ImageRole::DecodedInput).unwrap(),1);
        assert_eq!(sender.live_count(ImageRole::EncodedInput).unwrap(),1);
        assert!(sender.live_bytes().unwrap()<super::model::MAX_PIXELS as u64,"tiny PNG has no full-limit minimum allocation");
        let target=images(ImageReceiver::limits().unwrap(),512*1024*1024);
        let (ended,mut end)=tokio::sync::oneshot::channel();
        let mut owner=receiver(target.clone());owner.lifetime=Some(Arc::new(End(Mutex::new(Some(ended)))));
        let (mut output,mut input)=tokio::io::duplex(64);
        let (sent,received)=tokio::join!(wire::send_prepared(&mut output,&encoded),wire::receive_reply(&mut input,&owner));
        sent.unwrap();let (_,image)=received.unwrap();let image=image.unwrap();
        assert_eq!(image.image().data(),&[1,2,3,255]);
        assert_eq!(target.live_bytes().unwrap(),4);
        assert_eq!(target.live_count(ImageRole::FilterScratch).unwrap(),0);
        drop(owner);assert!(end.try_recv().is_err());
        let alias=image.clone();drop(image);assert!(end.try_recv().is_err());
        drop(alias);end.await.unwrap();assert_eq!(target.live_bytes().unwrap(),0);
        drop(encoded);assert_eq!(sender.live_bytes().unwrap(),0);
    });
}
#[test]
fn batches_task_login_image_declared_png_larger_than_raw_and_refusal_before_body() {
    runtime().block_on(async {
        let png=png();assert!(png.len()>4);
        let target=images(ImageReceiver::limits().unwrap(),512*1024*1024);
        let owner=receiver(target.clone());
        let buffers=owner.prepare(&packet(),png.len()).unwrap();
        assert_eq!(target.live_bytes().unwrap(),png.len() as u64+4+3);drop(buffers);
        let mut input=Cursor::new(wire_bytes(&png));
        let (_,image)=wire::receive_reply(&mut input,&owner).await.unwrap();drop(image);
        assert_eq!(target.live_bytes().unwrap(),0);
        let refused=receiver(images(ImageReceiver::limits().unwrap(),4));
        let mut input=Cursor::new(wire_bytes(&png));
        assert!(wire::receive_reply(&mut input,&refused).await.is_err());
        assert_eq!(input.get_ref().len()-input.position() as usize,png.len()+5,"no PNG header/body read after refused promise");
        let mut wrong=packet();wrong.reply.width=2;wrong.reply.pixel_bytes=8;
        let mut buffers=owner.prepare(&wrong,png.len()).unwrap();buffers.png.as_mut_slice().copy_from_slice(&png);
        assert!(image_receive::decode(wrong,buffers,owner.clone()).is_err());assert_eq!(target.live_bytes().unwrap(),0);
    });
}
#[test]
fn batches_task_login_image_cancelled_future_keeps_blocking_decoder_until_actual_end() {
    runtime().block_on(async {
        let admission=images(ImageReceiver::limits().unwrap(),512*1024*1024);
        let gate=Arc::new((Mutex::new(false),Condvar::new()));let gate_worker=gate.clone();
        let _release_on_failure=ReleaseGate(gate.clone());
        let cancelled=Arc::new(AtomicBool::new(false));let cancel_worker=cancelled.clone();
        let calls=AtomicUsize::new(0);let (started,ready)=tokio::sync::oneshot::channel();let started=Mutex::new(Some(started));
        let (ended,end)=tokio::sync::oneshot::channel();
        let mut owner=receiver(admission.clone());owner.lifetime=Some(Arc::new(End(Mutex::new(Some(ended)))));
        owner.cancelled=Arc::new(move || {
            if calls.fetch_add(1,Ordering::SeqCst)==1 {
                if let Some(started)=started.lock().unwrap().take(){let _=started.send(());}
                let lock=gate_worker.0.lock().unwrap();drop(gate_worker.1.wait_while(lock,|released|!*released).unwrap());
            }
            cancel_worker.load(Ordering::SeqCst)
        });
        let bytes=wire_bytes(&png());
        let task=tokio::spawn(async move {wire::receive_reply(&mut Cursor::new(bytes),&owner).await});
        tokio::time::timeout(std::time::Duration::from_secs(10),ready).await.unwrap().unwrap();
        task.abort();let _=task.await;cancelled.store(true,Ordering::SeqCst);
        assert!(admission.live_bytes().unwrap()>4,"blocking decoder still owns encoded/raw/row storage");
        {let mut released=gate.0.lock().unwrap();*released=true;gate.1.notify_all();}
        tokio::time::timeout(std::time::Duration::from_secs(10),end).await.unwrap().unwrap();
        assert_eq!(admission.live_bytes().unwrap(),0);
    });
}
