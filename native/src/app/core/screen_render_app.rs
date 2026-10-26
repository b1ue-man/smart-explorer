//! All native apps share one renderer lifecycle, including early inner returns.
use std::sync::{Arc,Mutex};
use eframe::egui;
use super::screen_render_owner::{RendererOwner,lock};
struct ScreenRenderApp<'a> {inner:Box<dyn eframe::App+'a>,owner:Arc<RendererOwner>,ctx:egui::Context}
impl eframe::App for ScreenRenderApp<'_> {
    fn update(&mut self,ctx:&egui::Context,frame:&mut eframe::Frame) {
        self.owner.service(ctx);self.inner.update(ctx,frame);
        // Post-update retirement is processed before eframe can enter this output's callbacks.
        // Uploads wait for the next pre-update boundary so selected receipts stay exact.
        self.owner.after_update(ctx);
    }
    fn save(&mut self,storage:&mut dyn eframe::Storage) {self.inner.save(storage);}
    fn on_exit(&mut self,gl:Option<&eframe::glow::Context>) {self.inner.on_exit(gl);self.owner.shutdown(&self.ctx,gl);}
    fn auto_save_interval(&self)->std::time::Duration {self.inner.auto_save_interval()}
    fn clear_color(&self,visuals:&egui::Visuals)->[f32;4] {self.inner.clear_color(visuals)}
    fn persist_egui_memory(&self)->bool {self.inner.persist_egui_memory()}
    fn raw_input_hook(&mut self,ctx:&egui::Context,input:&mut egui::RawInput) {self.inner.raw_input_hook(ctx,input);}
}
/// Each attempt has its own owner; return closes it before a fallback starts.
pub(crate) fn run_native<'a>(name:&str,options:eframe::NativeOptions,creator:eframe::AppCreator<'a>)->eframe::Result<()> {
    let shader=options.shader_version.clone();let dithering=options.dithering;
    let running=Arc::new(Mutex::new(None::<Arc<RendererOwner>>));let capture=running.clone();
    let result=eframe::run_native(name,options,Box::new(move|cc| {
        let owner=RendererOwner::install(cc,shader,dithering);*lock(&capture)=Some(owner.clone());
        let inner=creator(cc)?;
        Ok(Box::new(ScreenRenderApp {inner,owner,ctx:cc.egui_ctx.clone()}))
    }));
    if let Some(owner)=lock(&running).take() {owner.returned();}result
}
