//! CPU draw selection and receipts are tied to the exact owned resident.
use std::sync::Arc;
use eframe::{egui,egui_wgpu};
use super::screen_render_owner::{RendererOwner,RenderSlot,Native,Backend,lock,set_resident};
use super::screen_render_upload::{Package,Generation};
impl RendererOwner {
    /// Output generation does not clear this request; only actual GL execution can upload/draw.
    pub(crate) fn paint_cpu(self:&Arc<Self>,painter:&egui::Painter,rect:egui::Rect,slot:&Arc<RenderSlot>,
        receipt:impl Fn(super::screen_presented::PresentedFrame)->Option<super::screen_presented::Presented>)->bool {
        self.attach(slot);let backend=lock(&self.backend);let state=lock(&slot.state);
        if matches!(*backend,Backend::Glow(_)) {
            let busy=state.resident.as_ref().is_some_and(|r|!r.upload.drain.confirmed());
            let package=(if busy {state.resident.as_ref().map(|r|&r.package)} else {state.candidate.as_ref().or_else(||state.resident.as_ref().map(|r|&r.package))}).filter(|p|p.active()).cloned();
            let Some(Package::Cpu(image))=package else{return false;};
            let fallback_stamp=state.resident.as_ref().filter(|r|r.package.active()).and_then(|r|r.package.frame());
            let fallback=fallback_stamp.clone().and_then(&receipt);
            let receipt=image.frame.clone().and_then(receipt);let weak=Arc::downgrade(self);let slot=slot.clone();let clip=painter.clip_rect();
            painter.add(egui::PaintCallback {rect,callback:Arc::new(eframe::egui_glow::CallbackFn::new(move|info,_| {
                let Some(owner)=weak.upgrade() else{return;};if owner.closed()||!image.generation.active()||image.frame.as_ref().is_some_and(|frame|!frame.active()){return;}
                let mut backend=lock(&owner.backend);let Backend::Glow(gl)=&mut *backend else{return;};let mut s=lock(&slot.state);
                if s.retiring.is_some()||!image.generation.same(&s.generation){return;}
                if s.resident.as_ref().is_none_or(|r|!r.package.same_image(&Package::Cpu(image.clone()))) {
                    let selected=Package::Cpu(image.clone());
                    let stale=!s.candidate.as_ref().is_some_and(|p|p.same_image(&selected))
                        &&!s.resident.as_ref().is_some_and(|r|r.package.same_image(&selected));
                    let busy=s.resident.as_ref().is_some_and(|r|!r.upload.drain.confirmed());
                    let resize=s.resident.as_ref().is_some_and(|r|matches!(&r.native,Native::Glow(old) if old.size!=image.image.size));
                    // An output prepared before after_update must not bypass its
                    // resize admission or upload an already replaced candidate.
                    if stale||busy||resize {draw_fallback(gl,&s,rect,clip,info,&fallback_stamp,&fallback);return;}
                    let Ok(claims)=s.claim_upload(&selected) else{draw_fallback(gl,&s,rect,clip,info,&fallback_stamp,&fallback);return;};
                    let existing=s.resident.take().and_then(|r|match r.native {Native::Glow(image)=>Some(image),_=>None});
                    let native=gl.upload(existing,&image);let package=Package::Cpu(image.clone());let drain=gl.checkpoint();
                    if s.candidate.as_ref().is_some_and(|candidate|candidate.same_image(&package)) {s.candidate=None;}
                    set_resident(&mut s,Native::Glow(native),package,drain,claims);s.admission=None;
                }
                let Some(Native::Glow(native))=s.resident.as_ref().map(|r|&r.native) else{return;};
                gl.draw(native.id,rect,clip,info);if let Some(receipt)=&receipt {receipt();}
            }))});true
        } else {
            let Some(resident)=state.resident.as_ref().filter(|r|r.package.active()) else{return false;};
            let id=match resident.native {Native::Wgpu(ref t)=>t.id,#[cfg(test)] Native::Controlled(id,_)=>id,_=>return false};
            let generation=resident.package.generation().clone();let stamp=resident.package.frame();let receipt=stamp.clone().and_then(receipt);
            painter.image(id,rect,egui::Rect::from_min_max(egui::Pos2::ZERO,egui::Pos2::new(1.0,1.0)),egui::Color32::WHITE);
            if let Some(receipt)=receipt {
                let callback=CpuReceipt {slot:slot.clone(),generation,stamp,receipt};
                #[cfg(test)] if matches!(*backend,Backend::Controlled {..}) {
                    painter.add(egui::PaintCallback {rect,callback:Arc::new(callback)});return true;
                }
                painter.add(egui_wgpu::Callback::new_paint_callback(rect,callback));
            }true
        }
    }
}
fn draw_fallback(gl:&mut super::screen_render_glow::GlowBackend,s:&super::screen_render_upload::SlotState,
    rect:egui::Rect,clip:egui::Rect,info:egui::PaintCallbackInfo,
    stamp:&Option<super::screen_presented::PresentedFrame>,receipt:&Option<super::screen_presented::Presented>) {
    if let Some(resident)=s.resident.as_ref().filter(|r|r.package.active()) {
        if let Native::Glow(old)=&resident.native {
            gl.draw(old.id,rect,clip,info);
            if &resident.package.frame()==stamp {if let Some(receipt)=receipt {receipt();}}
        }
    }
}
struct CpuReceipt {slot:Arc<RenderSlot>,generation:Generation,stamp:Option<super::screen_presented::PresentedFrame>,receipt:super::screen_presented::Presented}
impl CpuReceipt {
    fn draw(&self)->bool {
        let s=lock(&self.slot.state);let matches=s.resident.as_ref().is_some_and(|r|r.package.active()
            &&r.package.generation().same(&self.generation)&&r.package.frame()==self.stamp);
        if matches {(self.receipt)();}matches
    }
}
impl egui_wgpu::CallbackTrait for CpuReceipt {
    fn paint(&self,_:egui::PaintCallbackInfo,_:&mut egui_wgpu::wgpu::RenderPass<'static>,_:&egui_wgpu::CallbackResources) {self.draw();}
}
pub(super) fn draw_receipt(slot:Arc<RenderSlot>,generation:Generation,stamp:Option<super::screen_presented::PresentedFrame>,receipt:super::screen_presented::Presented)->bool {
    CpuReceipt {slot,generation,stamp,receipt}.draw()
}

/// The controlled adapter delivers explicit IO events to the exact callback
/// emitted by LoginImage::show. This is not a WGPU/GL renderpass.
#[cfg(test)]
pub(super) fn draw_controlled_output(output:&egui::FullOutput)->usize {
    fn draw(shape:&egui::Shape)->usize {
        match shape {
            egui::Shape::Callback(callback)=>callback.callback.downcast_ref::<CpuReceipt>().map_or(0,|receipt|usize::from(receipt.draw())),
            egui::Shape::Vec(shapes)=>shapes.iter().map(draw).sum(),
            _=>0,
        }
    }
    output.shapes.iter().map(|shape|draw(&shape.shape)).sum()
}
