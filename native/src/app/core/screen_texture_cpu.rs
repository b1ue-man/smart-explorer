//! BGRA conversion and conflating CPU scaling feed one explicitly owned renderer slot.
use super::screen_image_memory::{CpuPixels, ImageAccess};
use super::screen_presented::{Presented, PresentedFrame};
use super::screen_render_owner::{get, RenderSlot};
use super::screen_render_upload::{Package, PreparedImage};
use super::screen_texture_worker::{Work, Worker};
use crate::screen::core::capture_image::CapturedImage;
use crate::screen::core::{
    frame_budget::FrameLimits, frame_memory::MemoryHold, pixel_convert::BgraImage,
};
use eframe::egui::{self, TextureId};
use std::sync::Arc;
pub(crate) struct CpuTexture {
    slot: Arc<RenderSlot>,
    source: Option<Arc<CpuPixels>>,
    seq: u64,
    prepared_seq: u64,
    source_frame: Option<PresentedFrame>,
    prepared: Option<[usize; 2]>,
    queued: Option<(u64, [usize; 2])>,
    worker: Option<Worker>,
    failed: bool,
    memory: MemoryHold,
    limits: Option<FrameLimits>,
    source_memory: MemoryHold,
    source_limits: Option<FrameLimits>,
    local: Option<ImageAccess>,
}
impl Default for CpuTexture {
    fn default() -> Self {
        Self {
            slot: Arc::default(),
            source: None,
            seq: 0,
            prepared_seq: 0,
            source_frame: None,
            prepared: None,
            queued: None,
            worker: None,
            failed: false,
            memory: None,
            limits: None,
            source_memory: None,
            source_limits: None,
            local: None,
        }
    }
}
impl CpuTexture {
    pub(crate) fn slot(&self) -> &Arc<RenderSlot> {
        &self.slot
    }
    #[cfg(test)]
    pub(super) fn set_test_local(
        &mut self,
        admission: Arc<crate::screen::core::image_admission::ImageAdmission>,
    ) {
        self.local = Some(ImageAccess::Local(admission));
    }
    pub(crate) fn reset(&mut self) {
        self.slot.watch_memory(&self.source_memory);
        self.slot.watch_memory(&self.memory);
        self.slot.reset();
        if let Some(worker) = &self.worker {
            worker.invalidate();
        }
        self.source = None;
        self.source_frame = None;
        self.source_memory = None;
        self.source_limits = None;
        self.memory = None;
        self.limits = None;
        self.prepared = None;
        self.queued = None;
        self.prepared_seq = 0;
        self.failed = false;
        super::screen_render_owner::notify_all();
    }
    pub(crate) fn bind_memory(&mut self, memory: MemoryHold, limits: Option<FrameLimits>) {
        self.memory = memory;
        self.limits = limits;
    }
    fn target(&self, ctx: &egui::Context, target: [usize; 2]) -> Option<[usize; 2]> {
        let target = super::screen_render_caps::checked_target(ctx, target)?;
        let size = crate::screen::core::pixel_convert::FrameSize::new(
            u32::try_from(target[0]).ok()?,
            u32::try_from(target[1]).ok()?,
        );
        let size = self.source_limits.unwrap_or(FrameLimits::LEGACY).fit(size);
        Some([size.width as usize, size.height as usize])
    }
    pub(crate) fn update(&mut self, ctx: &egui::Context, image: &BgraImage) {
        if super::screen_render_caps::check(ctx, image.size()).is_err()
            || self.limits.is_some_and(|l| l.check(image.size()).is_err())
        {
            self.reset();
            self.failed = true;
            return;
        }
        if let Some(owner) = get(ctx) {
            owner.attach(&self.slot);
        }
        let Ok(source) = CpuPixels::convert(image, ImageAccess::Contract(self.memory.clone()))
        else {
            self.failed = true;
            return;
        };
        if self
            .source
            .as_ref()
            .is_none_or(|old| old.size != source.size)
        {
            self.prepared = None;
        }
        self.source = Some(source);
        self.source_memory = self.memory.clone();
        self.source_limits = self.limits;
        self.source_frame = None;
        self.seq = self.seq.saturating_add(1);
        self.failed = false;
    }
    pub(crate) fn update_frame(
        &mut self,
        ctx: &egui::Context,
        image: &BgraImage,
        frame: PresentedFrame,
    ) {
        if self
            .source_frame
            .as_ref()
            .is_some_and(|old| !old.same_source(&frame))
        {
            // Keep the newly bound session promise while retiring the old generation.
            let memory = self.memory.clone();
            let limits = self.limits;
            self.reset();
            self.bind_memory(memory, limits);
        }
        self.update(ctx, image);
        if !self.failed {
            self.source_frame = Some(frame);
        }
    }
    /// The raw ViewerInput remains in CapturedImage until conversion has ended.
    /// The copy controller survives session resets at this stable pane slot.
    pub(crate) fn update_local_frame(
        &mut self,
        ctx: &egui::Context,
        image: &CapturedImage,
        frame: PresentedFrame,
    ) -> bool {
        if image.image_role() != Some(crate::screen::core::image_admission::ImageRole::ViewerInput)
            || image.image_admission().is_none()
            || super::screen_render_caps::check(ctx, image.size()).is_err()
            || super::screen_image_memory::login_limits()
                .check(image.size())
                .is_err()
        {
            return false;
        }
        if self.local.is_none() {
            self.local = ImageAccess::login().ok();
        }
        let Some(access) = self.local.clone() else {
            return false;
        };
        let Ok(source) = CpuPixels::convert(image.image(), access) else {
            return false;
        };
        if self
            .source_frame
            .as_ref()
            .is_some_and(|old| !old.same_source(&frame))
        {
            self.reset();
        }
        if let Some(owner) = get(ctx) {
            owner.attach(&self.slot);
        }
        if self
            .source
            .as_ref()
            .is_none_or(|old| old.size != source.size)
        {
            self.prepared = None;
        }
        self.source = Some(source);
        self.source_memory = None;
        self.source_limits = Some(super::screen_image_memory::login_limits());
        self.source_frame = Some(frame);
        self.seq = self.seq.saturating_add(1);
        self.failed = false;
        true
    }
    /// Login may redraw the same signed pixels after its existing focus/geometry epoch changes.
    pub(crate) fn reframe(&mut self, frame: PresentedFrame) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let memory = self.source_memory.clone();
        let limits = self.source_limits;
        self.reset();
        self.source = Some(source);
        self.source_memory = memory;
        self.source_limits = limits;
        self.source_frame = Some(frame);
        self.seq = self.seq.saturating_add(1);
    }
    fn upload(
        &mut self,
        ctx: &egui::Context,
        image: Arc<CpuPixels>,
        seq: u64,
        frame: Option<PresentedFrame>,
        memory: MemoryHold,
    ) {
        if self.target(ctx, image.size) != Some(image.size) {
            self.failed = true;
            return;
        }
        let Some(owner) = get(ctx).filter(|owner| owner.available()) else {
            self.failed = true;
            return;
        };
        owner.attach(&self.slot);
        if !self.slot.queue(Package::Cpu(PreparedImage {
            image: image.clone(),
            memory,
            frame,
            generation: self.slot.generation(),
            seq,
        })) {
            return;
        }
        self.prepared = Some(image.size);
        self.prepared_seq = seq;
        self.failed = false;
        ctx.request_repaint();
    }
    pub(crate) fn prepare(&mut self, ctx: &egui::Context, target: [usize; 2]) {
        if self
            .source_frame
            .as_ref()
            .is_some_and(|frame| !frame.active())
        {
            self.reset();
            return;
        }
        let Some(target) = self.target(ctx, target) else {
            self.failed = true;
            return;
        };
        let Some(source) = self.source.clone() else {
            return;
        };
        if source.size == target {
            if self.prepared_seq != self.seq || self.prepared != Some(target) {
                self.upload(
                    ctx,
                    source,
                    self.seq,
                    self.source_frame.clone(),
                    self.source_memory.clone(),
                );
            }
            return;
        }
        if !self.slot.accepting() {
            return;
        }
        if self.worker.is_none() {
            self.worker = Worker::new();
        }
        let Some(worker) = &self.worker else {
            self.failed = true;
            return;
        };
        let finished = worker.take();
        let generation = self.slot.generation();
        if self.queued != Some((self.seq, target)) {
            worker.submit(Work {
                source: source.clone(),
                target,
                seq: self.seq,
                frame: self.source_frame.clone(),
                ctx: ctx.clone(),
                memory: self.source_memory.clone(),
                generation: generation.clone(),
            });
            self.queued = Some((self.seq, target));
        }
        if let Some(done) = finished.filter(|d| {
            d.generation.active()
                && d.frame.as_ref().is_none_or(|frame| frame.active())
                && d.generation.same(&generation)
                && d.source == source.size
                && d.target == target
                && d.seq >= self.prepared_seq
                && d.frame
                    .as_ref()
                    .zip(self.source_frame.as_ref())
                    .is_none_or(|(a, b)| {
                        a.session == b.session && a.epoch == b.epoch && a.gop_id == b.gop_id
                    })
        }) {
            if let Some(image) = done.image {
                self.upload(ctx, image, done.seq, done.frame, done.memory);
            } else {
                self.queued = None;
                self.failed = self.slot.frame().is_none();
            }
        }
    }
    pub(crate) fn paint(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        receipt: impl Fn(PresentedFrame) -> Option<Presented>,
    ) -> bool {
        let healthy = !self.failed;
        get(painter.ctx()).is_some_and(|owner| {
            owner.paint_cpu(painter, rect, &self.slot, |frame| {
                if healthy {
                    receipt(frame)
                } else {
                    None
                }
            })
        }) && healthy
    }
    pub(crate) fn presented_frame(&self) -> Option<PresentedFrame> {
        self.slot.frame()
    }
    pub(crate) fn failed(&self) -> bool {
        self.failed
    }
    pub(crate) fn id(&self) -> Option<TextureId> {
        self.slot.id()
    }
}
impl Drop for CpuTexture {
    fn drop(&mut self) {
        self.reset();
        self.slot.close();
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;
    #[test]
    fn batches_task_b6_cpu_source_keeps_its_lease_when_next_session_is_bound() {
        let pool = Arc::new(crate::screen::core::frame_memory::MemoryPool::default());
        let memory = Arc::new(
            pool.reserve(None, crate::screen::core::frame_budget::MAX_WORKING_BYTES)
                .unwrap(),
        );
        let ctx = egui::Context::default();
        let image =
            BgraImage::new(crate::screen::core::pixel_convert::FrameSize::new(2, 2)).unwrap();
        let mut texture = CpuTexture::default();
        texture.bind_memory(Some(memory.clone()), Some(FrameLimits::LEGACY));
        texture.update(&ctx, &image);
        texture.bind_memory(None, None);
        drop(memory);
        assert!(texture.source.is_some());
        assert!(pool.reserve(None, 1024 * 1024).is_err());
        let work = Work {
            source: texture.source.as_ref().unwrap().clone(),
            target: [1, 1],
            seq: texture.seq,
            frame: None,
            ctx,
            memory: texture.source_memory.clone(),
            generation: texture.slot.generation(),
        };
        drop(texture);
        assert!(pool.reserve(None, 1024 * 1024).is_err());
        drop(work);
        assert!(pool.reserve(None, 1024 * 1024).is_ok());
    }
    #[test]
    fn batches_task_b6_login_epoch_reframe_aliases_pixels_and_retires_old_generation() {
        let ctx = egui::Context::default();
        let driver = super::super::screen_render_owner::ControlledDriver::install(&ctx);
        let mut texture = CpuTexture::default();
        let image =
            BgraImage::new(crate::screen::core::pixel_convert::FrameSize::new(2, 2)).unwrap();
        let frame = super::super::screen_presented::PresentedFrame::default();
        texture.update_frame(&ctx, &image, frame.clone());
        texture.prepare(&ctx, [2, 2]);
        driver.upload();
        driver.complete_upload(texture.slot());
        let pixels = Arc::downgrade(texture.source.as_ref().unwrap());
        let slot = texture.slot().clone();
        let mut next = frame;
        next.epoch = 1;
        texture.reframe(next);
        assert!(std::sync::Weak::ptr_eq(
            &pixels,
            &Arc::downgrade(texture.source.as_ref().unwrap())
        ));
        assert!(Arc::ptr_eq(&slot, texture.slot()));
        assert!(texture.presented_frame().is_none());
        driver.owner.after_update(&ctx);
        driver.complete_retirement(&slot);
    }
}
