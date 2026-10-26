//! BGRA texture path with one conflating reconstruction worker. Native pixels
//! upload directly; area/cubic scaling never waits on the GUI thread.
use std::sync::{Arc,Condvar,Mutex,PoisonError};
use eframe::egui::{self,Color32,ColorImage,TextureHandle,TextureId,TextureOptions};
use crate::screen::core::pixel_convert::BgraImage;

pub(crate) fn color_image(image: &BgraImage) -> ColorImage {
    let size = image.size();
    let pixels = image.data().chunks_exact(4)
        .map(|bgra| Color32::from_rgb(bgra[2],bgra[1],bgra[0])).collect();
    ColorImage { size:[size.width as usize,size.height as usize],pixels }
}
struct Work { source:Arc<ColorImage>, target:[usize;2], seq:u64, frame:Option<super::screen_presented::PresentedFrame>, ctx:egui::Context }
struct Finished { source:[usize;2], target:[usize;2], seq:u64, frame:Option<super::screen_presented::PresentedFrame>, image:Option<ColorImage> }
#[derive(Default)]
struct Slots { pending:Option<Work>, finished:Option<Finished>, stopped:bool }
struct Worker { shared:Arc<(Mutex<Slots>,Condvar)> }
impl Worker {
    fn new() -> Option<Self> {
        let shared = Arc::new((Mutex::new(Slots::default()),Condvar::new()));
        let thread = shared.clone();
        let _ = std::thread::Builder::new().name("screen-scale".into()).spawn(move || loop {
            let (lock,ready) = &*thread;
            let work = {
                let mut slots = lock.lock().unwrap_or_else(PoisonError::into_inner);
                while !slots.stopped && slots.pending.is_none() {
                    slots = ready.wait(slots).unwrap_or_else(PoisonError::into_inner);
                }
                if slots.stopped { return; }
                slots.pending.take()
            };
            let Some(work) = work else { continue; };
            let image = super::screen_scaling::resample_with_cancel(&work.source,work.target,&||
                thread.0.lock().unwrap_or_else(PoisonError::into_inner).stopped);
            let mut slots = lock.lock().unwrap_or_else(PoisonError::into_inner);
            if slots.stopped { return; }
            slots.finished = Some(Finished { source:work.source.size,target:work.target,seq:work.seq,frame:work.frame,image });
            drop(slots);
            work.ctx.request_repaint();
        }).ok()?;
        Some(Self { shared })
    }
    fn submit(&self, work:Work) {
        let (lock,ready) = &*self.shared;
        lock.lock().unwrap_or_else(PoisonError::into_inner).pending = Some(work);
        ready.notify_one();
    }
    fn take(&self) -> Option<Finished> {
        self.shared.0.lock().unwrap_or_else(PoisonError::into_inner).finished.take()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let (lock,ready) = &*self.shared;
        let mut slots = lock.lock().unwrap_or_else(PoisonError::into_inner);
        slots.stopped = true;
        slots.pending = None;
        ready.notify_one();
    }
}
#[derive(Default)]
pub(crate) struct CpuTexture {
    handle:Option<TextureHandle>,
    source:Option<Arc<ColorImage>>,
    seq:u64,
    presented:u64,
    source_frame:Option<super::screen_presented::PresentedFrame>,
    presented_frame:Option<super::screen_presented::PresentedFrame>,
    prepared:Option<[usize;2]>,
    queued:Option<(u64,[usize;2])>,
    worker:Option<Worker>,
    failed:bool,
}
impl CpuTexture {
    pub(crate) fn update(&mut self, ctx:&egui::Context, image:&BgraImage) {
        if super::screen_render_caps::check(ctx,image.size()).is_err() {
            self.source=None; self.prepared=None; self.presented_frame=None; self.source_frame=None;
            self.seq=self.seq.saturating_add(1); self.failed=true; return;
        }
        let source = color_image(image);
        if self.source.as_ref().is_none_or(|old| old.size != source.size) { self.prepared = None; }
        self.source = Some(Arc::new(source));
        self.source_frame = None;
        self.seq = self.seq.saturating_add(1);
        self.failed = false;
    }
    pub(crate) fn update_frame(&mut self, ctx:&egui::Context, image:&BgraImage, frame:super::screen_presented::PresentedFrame) {
        self.update(ctx,image);
        if !self.failed {self.source_frame = Some(frame);}
    }
    fn upload(&mut self, ctx:&egui::Context, image:ColorImage, seq:u64, frame:Option<super::screen_presented::PresentedFrame>) {
        if super::screen_render_caps::checked_target(ctx,image.size)!=Some(image.size) {
            self.prepared=None; self.presented_frame=None; self.failed=true; return;
        }
        self.prepared = Some(image.size);
        self.presented = seq;
        self.presented_frame = frame;
        self.failed = false;
        match self.handle.as_mut() {
            Some(handle) => handle.set(image,TextureOptions::NEAREST),
            None => self.handle = Some(ctx.load_texture("screen-viewer",image,TextureOptions::NEAREST)),
        }
    }
    pub(crate) fn prepare(&mut self, ctx:&egui::Context, target:[usize;2]) {
        let Some(target)=super::screen_render_caps::checked_target(ctx,target) else {self.failed=true; return;};
        let Some(source) = self.source.as_ref().cloned() else { return; };
        if source.size == target {
            if self.presented != self.seq || self.prepared != Some(target) {
                self.upload(ctx,(*source).clone(),self.seq,self.source_frame);
            }
            return;
        }
        if self.worker.is_none() { self.worker = Worker::new(); }
        let Some(worker) = self.worker.as_ref() else { self.failed = true; return; };
        let finished = worker.take();
        if self.queued != Some((self.seq,target)) {
            worker.submit(Work { source:source.clone(),target,seq:self.seq,frame:self.source_frame,ctx:ctx.clone() });
            self.queued = Some((self.seq,target));
        }
        if let Some(done) = finished.filter(|done| done.source == source.size && done.target == target && done.seq >= self.presented) {
            if let Some(image) = done.image { self.upload(ctx,image,done.seq,done.frame); }
            else { self.failed = true; }
        }
    }
    pub(crate) fn presented_frame(&self) -> Option<super::screen_presented::PresentedFrame> { self.presented_frame }
    pub(crate) fn failed(&self) -> bool { self.failed }
    pub(crate) fn id(&self) -> Option<TextureId> {
        self.prepared.and_then(|_| self.handle.as_ref().map(TextureHandle::id))
    }
}
