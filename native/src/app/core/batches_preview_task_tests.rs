//! The draw receipt is invoked explicitly: no GPU/runtime success is claimed
//! by this source fixture. The remote suite exercises this same gate once.
use super::*;
use crate::screen::core::{frame_budget::{FrameCapabilities,FrameContract},pixel_convert::BgraImage,
    visibility::{ViewerVisibility,VisibilityState,VisibleRect}};
use super::super::{screen_viewer_fake as fake,screen_session::{Picture,PictureData},screen_tile_face::preview_presented};
fn report(hidden:bool)->ViewerVisibility {
    ViewerVisibility {geometry_generation:0,state:if hidden {VisibilityState::Hidden} else {VisibilityState::Visible},
        minimized:hidden,rects:if hidden {Vec::new()} else {vec![VisibleRect {x:0,y:0,width:2,height:2}]}}
}
fn picture(frame:super::super::screen_presented::PresentedFrame)->Picture {
    Picture {seq:frame.frame_id,presented:frame,size:FrameSize::new(2,2),
        data:PictureData::Bgra(BgraImage::new(FrameSize::new(2,2)).unwrap()),memory:None}
}
#[test]
fn batches_task_b6_preview_resume_accepts_fresh_image_before_input_and_requires_draw() {
    let fake=fake::Fake::new(CapabilityDecision::Allow);
    let backend=fake.backend();let session=fake::open_preview(&fake,&backend);
    let caps=FrameCapabilities::resources(FrameSize::new(2,2));
    let mut hello=fake::hello(CapabilityDecision::Allow);
    hello.frame_contract=FrameContract::negotiate(Some(caps),Some(caps)).unwrap();
    session.shared.update(|info|{info.hello=Some(hello);info.state=SessionState::Streaming;});
    let ctx=egui::Context::default();let mut texture=CpuTexture::default();
    let old=session.shared.begin_frame(1,1);
    session.set_visibility(report(true));
    assert!(accept_preview_picture(&session,&mut texture,&ctx,picture(old.clone())).is_none());
    session.set_visibility(report(false));
    let fresh=session.shared.begin_frame(2,10);
    assert!(!session.input_ready());
    assert!(accept_preview_picture(&session,&mut texture,&ctx,picture(fresh.clone())).is_none());
    session.shared.visibility_ack(2,10);
    assert!(accept_preview_picture(&session,&mut texture,&ctx,picture(old.clone())).is_none());
    assert_eq!(accept_preview_picture(&session,&mut texture,&ctx,picture(fresh.clone())),Some(FrameSize::new(2,2)));
    assert!(!session.input_ready());
    assert!(preview_presented(&session,&texture,true).is_none()); // Not uploaded yet.
    texture.prepare(&ctx,[2,2]);
    assert!(texture.presented_frame().is_none());assert!(texture.failed());
    let driver=super::super::screen_render_owner::ControlledDriver::install(&ctx);
    texture.prepare(&ctx,[2,2]);assert!(texture.presented_frame().is_none());
    driver.upload();
    assert_eq!(texture.presented_frame(),Some(fresh.clone()));
    assert!(preview_presented(&session,&texture,false).is_none());
    let drawn=preview_presented(&session,&texture,true).unwrap();
    assert!(!session.input_ready());assert!(driver.draw(texture.slot(),fresh.clone(),drawn.clone()));assert!(session.input_ready());
    session.set_visibility(report(true));
    drawn();assert!(!session.input_ready());
    texture.reset();driver.owner.after_update(&ctx);
    assert!(!driver.draw(texture.slot(),fresh,drawn));
    assert!(preview_presented(&session,&texture,true).is_none());
    assert_eq!(fake.opens(),1);
}
