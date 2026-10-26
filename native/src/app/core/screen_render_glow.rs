//! The pinned egui painter owns every GL image ID and performs its own cleanup.
use std::sync::Arc;
use eframe::{egui,egui_glow::{self,glow::{self,HasContext}}};
use super::screen_render_upload::{Drain,PreparedImage};
// GLsync is an opaque handle. Only these GL-thread methods reconstruct it;
// neither the poll service nor an admission thread calls a GL function.
struct SyncPoint(usize);
pub(crate) struct GlowBackend { painter:egui_glow::Painter, next:u64, fences:Vec<(SyncPoint,Drain)> }
impl GlowBackend {
    pub(crate) fn new(gl:Arc<glow::Context>,shader:Option<egui_glow::ShaderVersion>,dithering:bool)->Result<Self,String> {
        egui_glow::Painter::new(gl,"",shader,dithering).map(|painter|Self {painter,next:1,fences:Vec::new()}).map_err(|e|e.to_string())
    }
    pub(crate) fn context_matches(&self,gl:&glow::Context)->bool {std::ptr::eq(gl,self.painter.gl().as_ref())}
    pub(crate) fn texture_limit(&self)->u32 {self.painter.max_texture_side().min(u32::MAX as usize) as u32}
    pub(crate) fn upload(&mut self,existing:Option<egui::TextureId>,image:&PreparedImage)->egui::TextureId {
        let id=existing.unwrap_or_else(||{let id=egui::TextureId::Managed(self.next);self.next+=1;id});
        self.painter.set_texture(id,&egui::epaint::ImageDelta::full(image.image.clone(),egui::TextureOptions::NEAREST));id
    }
    pub(crate) fn draw(&mut self,id:egui::TextureId,rect:egui::Rect,clip:egui::Rect,info:egui::PaintCallbackInfo) {
        let mut mesh=egui::Mesh::with_texture(id);
        mesh.add_rect_with_uv(rect,egui::Rect::from_min_max(egui::Pos2::ZERO,egui::Pos2::new(1.0,1.0)),egui::Color32::WHITE);
        self.painter.paint_primitives(info.screen_size_px,info.pixels_per_point,&[egui::ClippedPrimitive {
            clip_rect:clip.intersect(info.clip_rect),primitive:egui::epaint::Primitive::Mesh(mesh),
        }]);
    }
    pub(crate) fn retire(&mut self,id:egui::TextureId)->Drain {
        self.painter.free_texture(id);
        self.checkpoint()
    }
    pub(crate) fn checkpoint(&mut self)->Drain {
        let drain=Drain::default();let gl=self.painter.gl();let version=gl.version();
        let sync=if version.is_embedded {version.major>=3} else {version.major>3||(version.major==3&&version.minor>=2)||gl.supported_extensions().contains("GL_ARB_sync")};
        if sync {
            match unsafe {gl.fence_sync(glow::SYNC_GPU_COMMANDS_COMPLETE,0)} {
                Ok(fence)=>{unsafe{gl.flush();}self.fences.push((SyncPoint(fence.0 as usize),drain.clone()));}
                Err(_)=>{unsafe{gl.finish();}drain.confirm();}
            }
        } else {unsafe{gl.finish();}drain.confirm();}
        drain
    }
    pub(crate) fn service(&mut self) {
        let gl=self.painter.gl();
        self.fences.retain(|(fence,drain)| {
            let result=unsafe{gl.client_wait_sync(glow::NativeFence(fence.0 as _),0,0)};
            if result==glow::ALREADY_SIGNALED||result==glow::CONDITION_SATISFIED {
                unsafe{gl.delete_sync(glow::NativeFence(fence.0 as _));}drain.confirm();false
            } else {true} // WAIT_FAILED is not completion.
        });
    }
    pub(crate) fn shutdown(&mut self) {
        self.painter.destroy();let gl=self.painter.gl();unsafe{gl.finish();}
        for (fence,drain) in self.fences.drain(..) {unsafe{gl.delete_sync(glow::NativeFence(fence.0 as _));}drain.confirm();}
    }
}
