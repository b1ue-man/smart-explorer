//! Explicit IO events exercise the productive slot/receipt boundary, not a graphics driver.
use super::*;
use eframe::egui_wgpu;
use super::super::screen_render_upload::Generation;
use crate::screen::core::frame_memory::MemoryHold;
use super::super::{screen_texture_cpu::CpuTexture,screen_presented::{PresentedFrame,SessionToken},screen_session::Shared};
use crate::screen::core::{frame_budget::{FrameLimits,MAX_WORKING_BYTES},frame_memory::MemoryPool,pixel_convert::{BgraImage,FrameSize}};
use std::sync::atomic::{AtomicUsize,Ordering};
/// Test events replace backend IO only; slot admission, retirement and draw checks stay productive.
#[cfg(test)]
pub(crate) struct ControlledDriver {pub(crate) owner:Arc<RendererOwner>,ctx:egui::Context}
#[cfg(test)]
impl ControlledDriver {
    pub(crate) fn install(ctx:&egui::Context)->Self {
        let owner=Arc::new(RendererOwner {backend:Mutex::new(Backend::Controlled {upload:false}),slots:Mutex::new(Vec::new()),closed:false.into(),limit:16384,yuv:false});
        ctx.data_mut(|d|d.insert_temp(egui::Id::new(KEY),Arc::downgrade(&owner)));Self {owner,ctx:ctx.clone()}
    }
    pub(crate) fn upload(&self) {if let Backend::Controlled {upload}=&mut *lock(&self.owner.backend) {*upload=true;}
        self.owner.service(&self.ctx);if let Backend::Controlled {upload}=&mut *lock(&self.owner.backend) {*upload=false;}}
    pub(crate) fn complete_upload(&self,slot:&RenderSlot) {
        let s=lock(&slot.state);let drain=s.resident.as_ref().map(|r|r.upload.drain.clone()).or_else(||s.retiring.as_ref().map(|r|r.upload.drain.clone()));drop(s);
        if let Some(drain)=drain {drain.confirm();}self.owner.after_update(&self.ctx);
    }
    pub(crate) fn complete_retirement(&self,slot:&RenderSlot) {
        let drain=lock(&slot.state).retiring.as_ref().map(|r|r.drain.clone());if let Some(drain)=drain {drain.confirm();}self.owner.after_update(&self.ctx);
    }
    pub(crate) fn draw_output(&self,output:&egui::FullOutput)->usize {super::super::screen_render_cpu::draw_controlled_output(output)}
    pub(crate) fn draw(&self,slot:&Arc<RenderSlot>,stamp:PresentedFrame,receipt:super::super::screen_presented::Presented)->bool {
        super::super::screen_render_cpu::draw_receipt(slot.clone(),slot.generation(),Some(stamp),receipt)
    }
}

fn image()->BgraImage {BgraImage::new(FrameSize::new(2,2)).unwrap()}
fn stamp(id:u64)->PresentedFrame {PresentedFrame {session:SessionToken::default(),validity:Generation::default(),login:None,epoch:1,gop_id:1,frame_id:id}}
fn texture(ctx:&egui::Context,memory:MemoryHold)->CpuTexture {let mut t=CpuTexture::default();t.bind_memory(memory,Some(FrameLimits::LEGACY));t.update_frame(ctx,&image(),stamp(1));t.prepare(ctx,[2,2]);t}
#[test]
fn batches_task_b6_renderer_no_context_prepare_never_claims_upload_or_draw() {
    let ctx=egui::Context::default();let t=texture(&ctx,None);
    assert!(t.failed());assert!(t.id().is_none());assert!(t.presented_frame().is_none());
}
#[test]
fn batches_task_b6_renderer_pending_upload_and_discarded_output_keep_exact_ownership() {
    let ctx=egui::Context::default();let driver=ControlledDriver::install(&ctx);
    let pool=Arc::new(MemoryPool::default());let memory=Arc::new(pool.reserve(None,MAX_WORKING_BYTES).unwrap());
    let mut t=texture(&ctx,Some(memory.clone()));drop(memory);
    assert!(t.presented_frame().is_none());assert!(lock(&t.slot().state).candidate.is_some());
    // A FullOutput can be discarded before the backend accepts its callback.
    let output=ctx.run(egui::RawInput::default(),|ctx| {egui::CentralPanel::default().show(ctx,|ui| {assert!(!t.paint(ui.painter(),ui.max_rect(),|_|None));});});
    drop(output);assert!(t.presented_frame().is_none());
    driver.upload();let slot=t.slot().clone();assert!(t.presented_frame().is_some());
    let held=lock(&slot.state).resident.as_ref().unwrap().upload.drain.clone();assert!(!held.confirmed());
    t.reset();driver.owner.after_update(&ctx);drop(t);
    assert!(pool.reserve(None,1024*1024).is_err());
    driver.complete_retirement(&slot);assert!(pool.reserve(None,1024*1024).is_err(),"retirement alone cannot bless an unconfirmed upload");
    driver.complete_upload(&slot);assert!(pool.reserve(None,MAX_WORKING_BYTES).is_ok());
}
#[test]
fn batches_task_b6_renderer_last_draw_requires_new_retirement_proof() {
    let ctx=egui::Context::default();let driver=ControlledDriver::install(&ctx);let mut t=texture(&ctx,None);
    driver.upload();driver.complete_upload(t.slot());let frame=t.presented_frame().unwrap();
    let calls=Arc::new(AtomicUsize::new(0));let count=calls.clone();let receipt:super::super::screen_presented::Presented=Arc::new(move||{count.fetch_add(1,Ordering::SeqCst);});
    assert!(driver.draw(t.slot(),frame.clone(),receipt.clone()));assert_eq!(calls.load(Ordering::SeqCst),1);
    let upload=lock(&t.slot().state).resident.as_ref().unwrap().upload.drain.clone();assert!(upload.confirmed());
    t.reset();driver.owner.after_update(&ctx);assert!(lock(&t.slot().state).retiring.is_some());
    assert!(!lock(&t.slot().state).retiring.as_ref().unwrap().drain.confirmed(),"the last upload is not the last draw's fence");
    assert!(!driver.draw(t.slot(),frame,receipt));assert_eq!(calls.load(Ordering::SeqCst),1);
    driver.complete_retirement(t.slot());assert!(lock(&t.slot().state).retiring.is_none());
}
#[test]
fn batches_task_b6_renderer_session_identity_and_conflation_survive_equal_frame_numbers() {
    let a=Shared::new(false,0);let b=Shared::new(false,0);let one=a.begin_frame(1,1);let two=b.begin_frame(1,1);
    assert_eq!((one.epoch,one.gop_id,one.frame_id),(two.epoch,two.gop_id,two.frame_id));assert_ne!(one.session,two.session);
    let ctx=egui::Context::default();let driver=ControlledDriver::install(&ctx);let mut t=CpuTexture::default();
    t.update_frame(&ctx,&image(),one.clone());t.prepare(&ctx,[2,2]);driver.upload();
    for id in 2..100 {let mut frame=one.clone();frame.frame_id=id;t.update_frame(&ctx,&image(),frame);t.prepare(&ctx,[2,2]);driver.upload();}
    let s=lock(&t.slot().state);assert_eq!(s.resident.as_ref().unwrap().package.frame(),Some(one));assert_eq!(s.candidate.as_ref().unwrap().frame().unwrap().frame_id,99);assert!(s.retiring.is_none());drop(s);
    t.update_frame(&ctx,&image(),two.clone());t.prepare(&ctx,[2,2]);driver.owner.after_update(&ctx);
    assert!(t.presented_frame().is_none());driver.complete_upload(t.slot());driver.complete_retirement(t.slot());driver.upload();assert_eq!(t.presented_frame(),Some(two));
}
#[test]
fn batches_task_b6_renderer_common_reset_and_app_shutdown_preserve_retirement_ownership() {
    for transition in ["common-reset","app-exit"] {
        let ctx=egui::Context::default();let driver=ControlledDriver::install(&ctx);let pool=Arc::new(MemoryPool::default());
        let memory=Arc::new(pool.reserve(None,MAX_WORKING_BYTES).unwrap());let mut t=texture(&ctx,Some(memory.clone()));drop(memory);
        driver.upload();driver.complete_upload(t.slot());let slot=t.slot().clone();
        if transition=="app-exit" {t.reset();driver.owner.shutdown(&ctx,None);}else{t.reset();driver.owner.after_update(&ctx);}
        drop(t);assert!(pool.reserve(None,1024*1024).is_err(),"{transition}: callback/UI lifetime was mistaken for GPU completion");
        driver.complete_retirement(&slot);assert!(pool.reserve(None,MAX_WORKING_BYTES).is_ok(),"{transition}");
        driver.owner.returned();assert!(lock(&driver.owner.slots).is_empty());
    }
}
#[path="batches_renderer_wgpu_task_tests.rs"] mod wgpu_tests;
#[path="batches_renderer_glow_task_tests.rs"] mod glow_tests;
#[path="batches_renderer_ui_task_tests.rs"] mod ui_tests;
#[test]
fn batches_task_b6_renderer_decoder_invalidation_rejects_already_prepared_output() {
    let ctx=egui::Context::default();let driver=ControlledDriver::install(&ctx);let shared=Shared::new(false,0);let frame=shared.begin_frame(1,1);let mut t=CpuTexture::default();
    t.update_frame(&ctx,&image(),frame.clone());t.prepare(&ctx,[2,2]);shared.invalidate_presentation();assert!(!frame.active());driver.upload();
    assert!(t.presented_frame().is_none());assert!(lock(&t.slot().state).candidate.is_none());
}
#[test]
fn batches_task_b6_renderer_concrete_texture_limit_preserves_native_4k_before_first_input() {
    let ctx=egui::Context::default();let _driver=ControlledDriver::install(&ctx);
    assert!(super::super::screen_render_caps::check(&ctx,FrameSize::new(3840,2160)).is_ok());
    assert_eq!(super::super::screen_render_caps::checked_target(&ctx,[3840,2160]),Some([3840,2160]));
    assert_eq!(crate::screen::core::frame_budget::VERSIONED_WORKING_IMAGES,24);
    assert_eq!(crate::screen::core::frame_budget::MAX_SESSION_WORKING_BYTES,864*1024*1024);
}
#[test]
fn batches_task_b6_renderer_stream_pause_keeps_last_pixels_but_revokes_receipt() {
    use crate::screen::core::session_state::SessionState;
    let shared=Shared::new(false,0);shared.update(|info|info.state=SessionState::Streaming);let frame=shared.begin_frame(1,1);
    assert!(shared.can_present(&frame));shared.suspend_presentation();assert!(frame.active());assert!(!shared.can_present(&frame));
    shared.cancel_presentation();assert!(!frame.active());
}
#[test]
fn batches_task_b6_renderer_return_closes_unconfirmed_instance_without_on_exit() {
    let ctx=egui::Context::default();let driver=ControlledDriver::install(&ctx);let pool=Arc::new(MemoryPool::default());
    let memory=Arc::new(pool.reserve(None,MAX_WORKING_BYTES).unwrap());let t=texture(&ctx,Some(memory.clone()));drop(memory);driver.upload();let slot=t.slot().clone();drop(t);
    driver.owner.after_update(&ctx);assert!(pool.reserve(None,1024*1024).is_err());assert!(lock(&slot.state).retiring.is_some());
    // This explicit test event represents concrete native-instance return, never a timeout or App drop.
    driver.owner.returned();assert!(!driver.owner.available());assert!(lock(&driver.owner.slots).is_empty());assert!(pool.reserve(None,MAX_WORKING_BYTES).is_ok());
}
