//! Controlled backend/queue events; no GL driver, privileged input or SAS executes.
use super::*;
use crate::app::{screen_render_owner::{ControlledDriver,RenderSlot,lock},
    screen_render_upload::{Package,PreparedImage,Drain},screen_render_cpu::{finish_glow_upload,finish_glow_draw},
    screen_render_glow::{GlowImage,GlowUpload},screen_image_memory::{CpuPixels,ImageAccess,owner_limits}};
use crate::login::os::shared::viewer::RendererFixture;
use crate::screen::core::{capture_image::CapturedImage,input_model::InputEvent,
    image_admission::{ImageAdmission,ImageRole,LocalImagePolicy},frame_memory::{MemoryPool,CapacityCeiling}};
use eframe::egui_glow::{GlOperationError,glow};
use std::time::Duration;

fn fixture()->(RendererFixture,CapturedImage) {
    let ipc=crate::daemon::login_image_fixture(FrameSize::new(2,2)).unwrap();
    let raw=ipc.session.wait_fixture_frame(Duration::from_secs(3)).unwrap();
    let mut view=ipc.session.snapshot().view.unwrap();
    // Explicit controlled rights only. The IPC fixture never receives SAS.
    view.access.as_mut().unwrap().rights.sas=true;
    let session=RendererFixture::new(view,ipc.admission.clone());ipc.finish().unwrap();
    session.session.focus(true,true);session.complete_release();(session,raw.image)
}
fn images()->Arc<ImageAdmission> {
    ImageAdmission::in_test_pool(Arc::new(MemoryPool::default()),LocalImagePolicy {
        facts:None,ceiling:CapacityCeiling::bounded(512*1024*1024),
    },owner_limits().unwrap())
}
fn packet(slot:&RenderSlot,owner:&Arc<ImageAdmission>,raw:&CapturedImage,frame:PresentedFrame)->Package {
    let seq=frame.frame_id;
    Package::Cpu(PreparedImage {image:CpuPixels::convert(raw.image(),ImageAccess::Local(owner.clone())).unwrap(),
        memory:None,frame:Some(frame),generation:slot.generation(),seq})
}
fn uploaded(slot:&RenderSlot,package:Package) {
    let mut state=lock(&slot.state);let claims=state.claim_upload(&package).unwrap();let old=state.resident.take();
    assert!(finish_glow_upload(&mut state,package,claims,old,GlowUpload {
        image:GlowImage {id:egui::TextureId::Managed(1),size:[2,2]},result:Ok(()),drain:Drain::default(),
    },|_|panic!("a successful upload must not retire its reused texture")));
}
fn receipt(session:&Arc<ViewerSession>,frame:&PresentedFrame)->crate::app::screen_presented::Presented {
    let session=session.clone();let token=frame.login.as_ref().unwrap().clone();let epoch=frame.epoch;
    Arc::new(move||{session.presented(&token,epoch);})
}
fn text()->Vec<InputEvent> {vec![InputEvent::Text {text:"x".into()}]}
fn check_revoked(fixture:&RendererFixture,token:&FrameToken,epoch:u64) {
    let session=&fixture.session;let current=session.snapshot();
    assert!(current.presentation_epoch>epoch);assert!(current.presented_frame.is_none());assert!(!current.input_ready);
    assert!(session.input(token.clone(),epoch,text()).is_err());assert!(session.secure_attention(token.clone(),epoch).is_err());
    assert!(fixture.release_pending());assert!(!session.presented(token,epoch));
}

#[test]
fn batches_task_b6_login_renderer_overwrite_revokes_input_sas_and_queue_before_retire() {
    let (fixture,raw)=fixture();let session=&fixture.session;let owner=images();let identity=SessionToken::default();
    let ctx=egui::Context::default();let driver=ControlledDriver::install(&ctx);let slot=Arc::new(RenderSlot::default());driver.owner.attach(&slot);
    let epoch=session.snapshot().presentation_epoch;let a=fixture.frame(1,2,2);let first=stamp(session,&a,identity.clone(),epoch);
    uploaded(&slot,packet(&slot,&owner,&raw,first.clone()));driver.complete_upload(&slot);
    let old_callback=receipt(session,&first);assert!(driver.draw(&slot,first.clone(),old_callback.clone()));assert!(session.snapshot().input_ready);
    session.input(a.clone(),epoch,text()).unwrap();session.secure_attention(a.clone(),epoch).unwrap();
    let b=fixture.frame(2,2,2);let second=stamp(session,&b,identity,epoch);let package=packet(&slot,&owner,&raw,second.clone());
    let mut state=lock(&slot.state);let claims=state.claim_upload(&package).unwrap();let old=state.resident.take();
    assert!(!finish_glow_upload(&mut state,package,claims,old,GlowUpload {
        image:GlowImage {id:egui::TextureId::Managed(1),size:[2,2]},
        result:Err(GlOperationError::from_gl_code(glow::INVALID_OPERATION)),drain:Drain::default(),
    },|_|{check_revoked(&fixture,&a,epoch);Drain::default()}));drop(state);
    assert_eq!(fixture.queued_allowed(),Some(false));assert_eq!(fixture.queued_allowed(),Some(false));
    assert!(!first.active());assert!(!second.active());old_callback();check_revoked(&fixture,&a,epoch);
    assert!(!driver.draw(&slot,first,old_callback));
    assert_eq!(owner.live_count(ImageRole::ColorSource).unwrap(),2);
    assert_eq!(owner.live_count(ImageRole::RenderSurface).unwrap(),1);assert_eq!(owner.live_count(ImageRole::UploadStaging).unwrap(),1);
    driver.complete_retirement(&slot);assert_eq!(owner.live_count(ImageRole::ColorSource).unwrap(),2);
    driver.complete_upload(&slot);assert_eq!(owner.live_count(ImageRole::RenderSurface).unwrap(),0);
    driver.owner.returned();assert_eq!(owner.live_bytes().unwrap(),0);
}

#[test]
fn batches_task_b6_login_renderer_draw_failure_recovers_and_ignores_late_revocation() {
    let (fixture,raw)=fixture();let session=&fixture.session;let owner=images();let identity=SessionToken::default();
    let ctx=egui::Context::default();let driver=ControlledDriver::install(&ctx);let slot=Arc::new(RenderSlot::default());driver.owner.attach(&slot);
    let epoch=session.snapshot().presentation_epoch;let a=fixture.frame(1,2,2);let first=stamp(session,&a,identity.clone(),epoch);
    let late=stamp(session,&a,identity.clone(),epoch);
    uploaded(&slot,packet(&slot,&owner,&raw,first.clone()));driver.complete_upload(&slot);
    let old_callback=receipt(session,&first);assert!(driver.draw(&slot,first,old_callback.clone()));
    let b=fixture.frame(2,2,2);let second=stamp(session,&b,identity.clone(),epoch);
    uploaded(&slot,packet(&slot,&owner,&raw,second.clone()));
    assert!(!finish_glow_draw(&mut lock(&slot.state),Err(GlOperationError::from_gl_code(glow::INVALID_FRAMEBUFFER_OPERATION)),|_| {
        check_revoked(&fixture,&a,epoch);Drain::default()
    }));
    old_callback();assert!(!driver.draw(&slot,second,receipt(session,&late)));check_revoked(&fixture,&a,epoch);
    driver.complete_retirement(&slot);assert_eq!(owner.live_count(ImageRole::RenderSurface).unwrap(),1);
    driver.complete_upload(&slot);assert_eq!(owner.live_count(ImageRole::RenderSurface).unwrap(),0);
    fixture.complete_release();slot.reset();let fresh_epoch=session.snapshot().presentation_epoch;
    let fresh=stamp(session,&b,identity,fresh_epoch);uploaded(&slot,packet(&slot,&owner,&raw,fresh.clone()));driver.complete_upload(&slot);
    assert!(driver.draw(&slot,fresh.clone(),receipt(session,&fresh)));assert!(session.snapshot().input_ready);
    session.input(b.clone(),fresh_epoch,text()).unwrap();session.secure_attention(b.clone(),fresh_epoch).unwrap();
    assert_eq!(fixture.queued_allowed(),Some(true));assert_eq!(fixture.queued_allowed(),Some(true));
    late.validity.revoke();old_callback();assert_eq!(session.snapshot().presentation_epoch,fresh_epoch);
    assert!(session.snapshot().input_ready);assert_eq!(session.snapshot().presented_frame,Some(b));assert!(!fixture.release_pending());
    driver.owner.returned();assert_eq!(owner.live_bytes().unwrap(),0);
}

#[test]
fn batches_task_b6_login_renderer_context_quarantine_revokes_every_slot_without_freeing_claims() {
    let (a,raw_a)=fixture();let (b,raw_b)=fixture();let owner_a=images();let owner_b=images();
    let ctx=egui::Context::default();let driver=ControlledDriver::install(&ctx);let mut slots=Vec::new();
    for (fixture,raw,owner) in [(&a,&raw_a,&owner_a),(&b,&raw_b,&owner_b)] {
        let token=fixture.frame(1,2,2);let epoch=fixture.session.snapshot().presentation_epoch;
        let frame=stamp(&fixture.session,&token,SessionToken::default(),epoch);let slot=Arc::new(RenderSlot::default());driver.owner.attach(&slot);
        uploaded(&slot,packet(&slot,owner,raw,frame.clone()));driver.complete_upload(&slot);
        assert!(driver.draw(&slot,frame.clone(),receipt(&fixture.session,&frame)));slots.push((slot,frame,token,epoch));
    }
    driver.owner.quarantine_slots();
    for ((slot,frame,token,epoch),fixture) in slots.iter().zip([&a,&b]) {
        check_revoked(fixture,token,*epoch);assert!(!frame.active());assert!(slot.frame().is_none());
        assert!(!driver.draw(slot,frame.clone(),receipt(&fixture.session,frame)));
    }
    assert_eq!(owner_a.live_count(ImageRole::RenderSurface).unwrap(),1);assert_eq!(owner_b.live_count(ImageRole::RenderSurface).unwrap(),1);
    driver.owner.after_update(&ctx);
    assert_eq!(owner_a.live_count(ImageRole::RenderSurface).unwrap(),1);assert_eq!(owner_b.live_count(ImageRole::RenderSurface).unwrap(),1);
    for (slot,..) in slots {driver.complete_retirement(&slot);}
    assert_eq!(owner_a.live_bytes().unwrap(),0);assert_eq!(owner_b.live_bytes().unwrap(),0);
    driver.owner.returned();
}

#[test]
fn batches_task_b6_screen_renderer_failure_revokes_input_and_reuses_visibility_floor() {
    use crate::app::screen_session::{Shared,ViewerSession as ScreenSession};
    use crate::screen::core::{session_state::SessionState,visibility::{ViewerVisibility,VisibilityState}};
    for modern in [false,true] {
        let (fixture,raw)=fixture();drop(fixture);let owner=images();let ctx=egui::Context::default();let driver=ControlledDriver::install(&ctx);
        let shared=Shared::new(false,0);shared.update(|info|info.state=SessionState::Streaming);let session=ScreenSession::fixture(shared.clone());
        lock(&shared.resume).report(&ViewerVisibility {geometry_generation:7,state:VisibilityState::Visible,minimized:false,rects:Vec::new()},modern);
        if modern {shared.visibility_ack(7,10);}
        let first=shared.begin_frame(1,10);let slot=Arc::new(RenderSlot::default());driver.owner.attach(&slot);
        uploaded(&slot,packet(&slot,&owner,&raw,first.clone()));driver.complete_upload(&slot);
        assert!(driver.draw(&slot,first.clone(),Arc::new({let shared=shared.clone();let first=first.clone();move||{shared.presented(first.clone(),||true);}})));
        assert!(session.input_ready());assert!(!finish_glow_draw(&mut lock(&slot.state),Err(GlOperationError::from_gl_code(glow::INVALID_OPERATION)),|_|Drain::default()));
        assert!(!session.input_ready());assert!(!shared.presented(first.clone(),||panic!("stale presented wire frame")));
        let fresh=shared.begin_frame(1,11);assert_ne!(first.validity,fresh.validity);assert!(!session.input_ready());
        let mut below_floor=fresh.clone();below_floor.frame_id=9;assert_eq!(shared.can_present(&below_floor),!modern);
        assert!(shared.can_present(&fresh));assert!(shared.presented(fresh,||true));assert!(session.input_ready());
        driver.owner.returned();assert_eq!(owner.live_bytes().unwrap(),0);
    }
}

#[test]
fn batches_task_b6_login_renderer_failure_interrupts_input_and_sas_before_send() {
    for sas in [false,true] {
        let (fixture,_raw)=fixture();let session=&fixture.session;let token=fixture.frame(1,2,2);let epoch=session.snapshot().presentation_epoch;
        let frame=stamp(session,&token,SessionToken::default(),epoch);assert!(session.presented(&token,epoch));
        if sas {session.secure_attention(token.clone(),epoch).unwrap();} else {session.input(token.clone(),epoch,text()).unwrap();}
        assert!(fixture.begin_queued_exchange());frame.validity.revoke();
        assert!(matches!(session.snapshot().state.as_str(),"ended"|"stopping"));check_revoked(&fixture,&token,epoch);
    }
}

#[test]
fn batches_task_b6_login_image_renderer_failure_reframes_same_pixels_before_sas_recovers() {
    let (fixture,raw)=fixture();let owner=images();fixture.queue_image(1,raw);
    let ctx=egui::Context::default();let driver=ControlledDriver::install(&ctx);let mut image=LoginImage::default();image.texture.set_test_local(owner.clone());
    let show=|image:&mut LoginImage| {
        let mut input=egui::RawInput {focused:true,screen_rect:Some(Rect::from_min_size(egui::Pos2::ZERO,Vec2::splat(2.0))),..Default::default()};
        input.viewports.entry(egui::ViewportId::ROOT).or_default().focused=Some(true);
        ctx.run(input,|ctx|{egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx,|ui|image.show(ui,&fixture.session,false));})
    };
    drop(show(&mut image));driver.upload();driver.complete_upload(image.texture.slot());
    let output=show(&mut image);assert_eq!(driver.draw_output(&output),1);let before=fixture.session.snapshot();let token=before.presented_frame.unwrap();
    let stale=show(&mut image);let slot=image.texture.slot().clone();
    assert!(!finish_glow_draw(&mut lock(&slot.state),Err(GlOperationError::from_gl_code(glow::INVALID_OPERATION)),|_|Drain::default()));
    // The SAS button runs before image.show. Its old snapshot is rejected now.
    assert!(fixture.session.secure_attention(token.clone(),before.presentation_epoch).is_err());
    assert_eq!(driver.draw_output(&stale),0);assert!(fixture.session.snapshot().presented_frame.is_none());
    drop(show(&mut image));driver.owner.after_update(&ctx);driver.complete_retirement(&slot);
    driver.upload();driver.complete_upload(&slot);fixture.complete_release();
    let restored=show(&mut image);assert_eq!(driver.draw_output(&restored),1);let after=fixture.session.snapshot();
    assert!(after.presentation_epoch>before.presentation_epoch);assert_eq!(after.presented_frame,Some(token.clone()));
    fixture.session.secure_attention(token,after.presentation_epoch).unwrap();assert_eq!(fixture.queued_allowed(),Some(true));
    image.reset();drop(image);driver.owner.after_update(&ctx);driver.complete_retirement(&slot);driver.owner.returned();
    drop(output);drop(stale);drop(restored);assert_eq!(owner.live_bytes().unwrap(),0);
}

#[test]
fn batches_task_b6_login_successful_overwrite_keeps_drawn_queue_proofs() {
    let (fixture,raw)=fixture();let session=&fixture.session;let owner=images();let identity=SessionToken::default();
    let ctx=egui::Context::default();let driver=ControlledDriver::install(&ctx);let slot=Arc::new(RenderSlot::default());driver.owner.attach(&slot);
    let epoch=session.snapshot().presentation_epoch;let a=fixture.frame(1,2,2);
    for id in 1..=10 {
        let token=if id==1 {a.clone()} else {fixture.frame(id,2,2)};let frame=stamp(session,&token,identity.clone(),epoch);
        uploaded(&slot,packet(&slot,&owner,&raw,frame.clone()));driver.complete_upload(&slot);
        assert!(driver.draw(&slot,frame.clone(),receipt(session,&frame)));assert_eq!(session.snapshot().presentation_epoch,epoch);
        if id==2 {
            session.input(a.clone(),epoch,text()).unwrap();session.secure_attention(a.clone(),epoch).unwrap();
            assert_eq!(fixture.queued_allowed(),Some(true));assert_eq!(fixture.queued_allowed(),Some(true));
        }
    }
    assert!(session.input(a.clone(),epoch,text()).is_err());assert!(session.secure_attention(a,epoch).is_err());
    let old_epoch=epoch;let resized=fixture.frame(11,3,2);assert!(!session.presented(&resized,old_epoch));assert!(!session.snapshot().input_ready);
    driver.owner.returned();assert_eq!(owner.live_bytes().unwrap(),0);
}
