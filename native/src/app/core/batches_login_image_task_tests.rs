//! Real local claims at the production conversion/worker/slot boundary. No dummy W lease.
use super::super::{
    screen_presented::PresentedFrame,
    screen_render_owner::{lock, ControlledDriver},
    screen_render_upload::Generation,
    screen_texture_cpu::CpuTexture,
    screen_texture_worker::{TestGate, Work, Worker},
};
use super::*;
use crate::screen::core::{
    capture_admission::{CaptureGeneration, SourceKey},
    capture_image::{CaptureImageBuilder, CapturedImage},
    frame_memory::{CapacityCeiling, MemoryPool},
    image_admission::LocalImagePolicy,
    pixel_convert::FrameSize,
};
use eframe::egui;

fn admission(pool: &Arc<MemoryPool>, limits: ImageOwnerLimits) -> Arc<ImageAdmission> {
    ImageAdmission::in_test_pool(
        pool.clone(),
        LocalImagePolicy {
            facts: None,
            ceiling: CapacityCeiling::bounded(512 * 1024 * 1024),
        },
        limits,
    )
}
fn local(pool: &Arc<MemoryPool>) -> Arc<ImageAdmission> {
    admission(pool, owner_limits().unwrap())
}
fn raw(pool: &Arc<MemoryPool>, size: FrameSize) -> (CapturedImage, Arc<ImageAdmission>) {
    let owner = admission(
        pool,
        ImageOwnerLimits::new(2 * LOGIN_BYTES)
            .unwrap()
            .with_role(ImageRole::ViewerInput, LOGIN_BYTES, 2)
            .unwrap(),
    );
    let image = CaptureImageBuilder::new_local(
        size,
        FrameLimits::LEGACY,
        &owner,
        ImageRole::ViewerInput,
        SourceKey::new("login-image-fixture", "desktop").unwrap(),
        CaptureGeneration::new(1),
    )
    .unwrap()
    .freeze();
    (image, owner)
}
fn texture(ctx: &egui::Context, owner: &Arc<ImageAdmission>, image: &CapturedImage) -> CpuTexture {
    let mut texture = CpuTexture::default();
    texture.set_test_local(owner.clone());
    assert!(texture.update_local_frame(ctx, image, PresentedFrame::default()));
    texture.prepare(
        ctx,
        [image.size().width as usize, image.size().height as usize],
    );
    texture
}

#[test]
fn batches_task_b6_login_image_context_without_renderer_cannot_claim_gpu_success() {
    let pool = Arc::new(MemoryPool::default());
    let owner = local(&pool);
    let (raw, _) = raw(&pool, FrameSize::new(2, 2));
    let texture = texture(&egui::Context::default(), &owner, &raw);
    assert!(texture.failed());
    assert!(texture.id().is_none());
    assert!(texture.presented_frame().is_none());
    assert_eq!(owner.live_count(ImageRole::RenderSurface).unwrap(), 0);
    assert_eq!(owner.live_count(ImageRole::UploadStaging).unwrap(), 0);
    drop(raw);
    drop(texture);
    assert_eq!(owner.live_bytes().unwrap(), 0);
}

#[test]
fn batches_task_b6_login_image_conversion_has_distinct_raw_and_color_claims_and_keeps_old_on_backpressure(
) {
    let pool = Arc::new(MemoryPool::default());
    let owner = local(&pool);
    let (raw, receiver) = raw(&pool, FrameSize::new(2, 2));
    let ctx = egui::Context::default();
    let driver = ControlledDriver::install(&ctx);
    let mut texture = texture(&ctx, &owner, &raw);
    driver.upload();
    driver.complete_upload(texture.slot());
    let old = texture.presented_frame().unwrap();
    assert_eq!(receiver.live_bytes().unwrap(), 16);
    assert_eq!(owner.live_count(ImageRole::ColorSource).unwrap(), 1);
    let blockers = [
        owner.try_claim(ImageRole::ColorSource, 16).unwrap(),
        owner.try_claim(ImageRole::ColorSource, 16).unwrap(),
    ];
    let bytes = owner.live_bytes().unwrap();
    assert!(!texture.update_local_frame(&ctx, &raw, PresentedFrame::default()));
    assert_eq!(owner.live_bytes().unwrap(), bytes);
    assert_eq!(texture.presented_frame(), Some(old));
    drop(blockers);
    drop(raw);
    assert_eq!(receiver.live_count(ImageRole::ViewerInput).unwrap(), 0);
    assert_eq!(owner.live_count(ImageRole::ColorSource).unwrap(), 1);
    drop(texture);
    driver.owner.returned();
    assert_eq!(owner.live_bytes().unwrap(), 0);
}

#[test]
fn batches_task_b6_login_image_output_and_scratch_are_claimed_before_filter_allocation() {
    let pool = Arc::new(MemoryPool::default());
    let owner = local(&pool);
    let (raw, _) = raw(&pool, FrameSize::new(8, 8));
    let source = CpuPixels::convert(raw.image(), ImageAccess::Local(owner.clone())).unwrap();
    let scratch = owner.try_claim(ImageRole::FilterScratch, 1).unwrap();
    assert!(super::super::screen_scaling::resample_admitted(&source, [4, 4], &|| false).is_none());
    assert_eq!(owner.live_count(ImageRole::ScaledOutput).unwrap(), 0);
    drop(scratch);
    let outputs: Vec<_> = (0..3)
        .map(|_| {
            super::super::screen_scaling::resample_admitted(&source, [4, 4], &|| false).unwrap()
        })
        .collect();
    assert_eq!(owner.live_count(ImageRole::ScaledOutput).unwrap(), 3);
    assert!(super::super::screen_scaling::resample_admitted(&source, [4, 4], &|| false).is_none());
    drop(outputs);
    let saw_scratch = std::cell::Cell::new(false);
    let cancelled = super::super::screen_scaling::resample_admitted(&source, [4, 4], &|| {
        if owner.live_count(ImageRole::FilterScratch).unwrap() == 1 {
            saw_scratch.set(true);
            true
        } else {
            false
        }
    });
    assert!(saw_scratch.get());
    assert!(cancelled.is_none());
    assert_eq!(owner.live_count(ImageRole::FilterScratch).unwrap(), 0);
    assert_eq!(owner.live_count(ImageRole::ScaledOutput).unwrap(), 0);
}

#[test]
fn batches_task_b6_login_image_worker_cancel_retains_source_and_successful_unpublished_output_until_exit(
) {
    for after_resample in [false, true] {
        let pool = Arc::new(MemoryPool::default());
        let owner = local(&pool);
        let (raw, receiver) = raw(&pool, FrameSize::new(8, 8));
        let source = CpuPixels::convert(raw.image(), ImageAccess::Local(owner.clone())).unwrap();
        drop(raw);
        assert_eq!(receiver.live_bytes().unwrap(), 0);
        let gate = Arc::new(if after_resample {
            TestGate::after_resample()
        } else {
            TestGate::default()
        });
        let worker = Worker::new().unwrap();
        worker.set_gate(gate.clone());
        let generation = Generation::default();
        worker.submit(Work {
            source,
            target: [4, 4],
            seq: 1,
            frame: None,
            ctx: egui::Context::default(),
            memory: None,
            generation: generation.clone(),
        });
        gate.wait_entered();
        assert_eq!(owner.live_count(ImageRole::ColorSource).unwrap(), 1);
        assert_eq!(
            owner.live_count(ImageRole::ScaledOutput).unwrap(),
            u64::from(after_resample)
        );
        generation.revoke();
        worker.invalidate();
        drop(worker);
        assert_eq!(owner.live_count(ImageRole::ColorSource).unwrap(), 1);
        gate.release();
        gate.wait_exited();
        assert_eq!(owner.live_bytes().unwrap(), 0);
    }
}

#[test]
fn batches_task_b6_login_image_reset_keeps_pane_slot_until_explicit_retirement() {
    let pool = Arc::new(MemoryPool::default());
    let owner = local(&pool);
    let (raw, receiver) = raw(&pool, FrameSize::new(2, 2));
    let ctx = egui::Context::default();
    let driver = ControlledDriver::install(&ctx);
    let mut texture = texture(&ctx, &owner, &raw);
    drop(raw);
    assert_eq!(receiver.live_bytes().unwrap(), 0);
    let staging = owner.try_claim(ImageRole::UploadStaging, 16).unwrap();
    driver.upload();
    assert!(texture.id().is_none());
    assert_eq!(owner.live_count(ImageRole::RenderSurface).unwrap(), 0);
    drop(staging);
    let surfaces = [
        owner.try_claim(ImageRole::RenderSurface, 16).unwrap(),
        owner.try_claim(ImageRole::RenderSurface, 16).unwrap(),
    ];
    driver.upload();
    assert!(texture.id().is_none());
    assert_eq!(owner.live_count(ImageRole::UploadStaging).unwrap(), 0);
    drop(surfaces);
    driver.upload();
    assert_eq!(owner.live_count(ImageRole::RenderSurface).unwrap(), 1);
    assert_eq!(owner.live_count(ImageRole::UploadStaging).unwrap(), 1);
    let slot = texture.slot().clone();
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            texture.paint(ui.painter(), ui.max_rect(), |_| None);
        });
    });
    drop(output);
    texture.reset();
    assert!(Arc::ptr_eq(&slot, texture.slot()));
    driver.owner.after_update(&ctx);
    drop(texture);
    driver.complete_retirement(&slot);
    assert_eq!(owner.live_count(ImageRole::RenderSurface).unwrap(), 1);
    assert_eq!(owner.live_count(ImageRole::UploadStaging).unwrap(), 1);
    driver.complete_upload(&slot);
    assert_eq!(owner.live_bytes().unwrap(), 0);
}

#[test]
fn batches_task_b6_login_image_upload_completion_preserves_surface_and_reframe_keeps_original_color_claim(
) {
    let pool = Arc::new(MemoryPool::default());
    let owner = local(&pool);
    let (raw, _) = raw(&pool, FrameSize::new(2, 2));
    let ctx = egui::Context::default();
    let driver = ControlledDriver::install(&ctx);
    let mut texture = texture(&ctx, &owner, &raw);
    drop(raw);
    driver.upload();
    driver.complete_upload(texture.slot());
    assert_eq!(owner.live_count(ImageRole::UploadStaging).unwrap(), 0);
    assert_eq!(owner.live_count(ImageRole::RenderSurface).unwrap(), 1);
    let source = match &lock(&texture.slot().state)
        .resident
        .as_ref()
        .unwrap()
        .package
    {
        super::super::screen_render_upload::Package::Cpu(image) => Arc::downgrade(&image.image),
        _ => unreachable!(),
    };
    let mut next = texture.presented_frame().unwrap();
    next.epoch += 1;
    texture.reframe(next);
    texture.prepare(&ctx, [2, 2]);
    assert_eq!(owner.live_count(ImageRole::ColorSource).unwrap(), 1);
    assert!(source.upgrade().is_some());
    driver.owner.after_update(&ctx);
    driver.complete_retirement(texture.slot());
    driver.upload();
    driver.complete_upload(texture.slot());
    let current = match &lock(&texture.slot().state)
        .resident
        .as_ref()
        .unwrap()
        .package
    {
        super::super::screen_render_upload::Package::Cpu(image) => Arc::downgrade(&image.image),
        _ => unreachable!(),
    };
    assert!(std::sync::Weak::ptr_eq(&source, &current));
    driver.owner.shutdown(&ctx, None);
    drop(texture);
    assert_eq!(owner.live_count(ImageRole::RenderSurface).unwrap(), 1);
    driver.owner.returned();
    assert_eq!(owner.live_bytes().unwrap(), 0);
}

#[test]
fn batches_task_b6_login_image_resize_keeps_old_draw_until_exact_replacement_claims_exist() {
    use super::super::screen_render_upload::{Package, PreparedImage};
    let pool = Arc::new(MemoryPool::default());
    let owner = local(&pool);
    let (raw, _) = raw(&pool, FrameSize::new(4, 4));
    let ctx = egui::Context::default();
    let driver = ControlledDriver::install(&ctx);
    let texture = texture(&ctx, &owner, &raw);
    drop(raw);
    driver.upload();
    driver.complete_upload(texture.slot());
    let slot = texture.slot().clone();
    let old = slot.frame().unwrap();
    let source = match &lock(&slot.state).resident.as_ref().unwrap().package {
        Package::Cpu(p) => p.image.clone(),
        _ => unreachable!(),
    };
    let reduced =
        super::super::screen_scaling::resample_admitted(&source, [2, 2], &|| false).unwrap();
    let mut frame = old.clone();
    frame.frame_id += 1;
    let package = Package::Cpu(PreparedImage {
        image: reduced,
        memory: None,
        frame: Some(frame.clone()),
        generation: slot.generation(),
        seq: 2,
    });
    assert!(slot.queue(package.clone()));
    let blocked = owner.try_claim(ImageRole::UploadStaging, 16).unwrap();
    driver.upload();
    assert_eq!(slot.frame(), Some(old));
    assert!(lock(&slot.state).retiring.is_none());
    drop(blocked);
    driver.upload();
    assert_eq!(owner.live_count(ImageRole::RenderSurface).unwrap(), 2);
    assert_eq!(owner.live_count(ImageRole::UploadStaging).unwrap(), 1);
    assert!(
        !slot.queue(package.clone()),
        "an admitted replacement keeps its original target and source"
    );
    driver.complete_retirement(&slot);
    driver.upload();
    driver.complete_upload(&slot);
    assert_eq!(slot.frame(), Some(frame));
    assert_eq!(owner.live_count(ImageRole::RenderSurface).unwrap(), 1);
    assert_eq!(owner.live_count(ImageRole::UploadStaging).unwrap(), 0);
    drop(source);
    drop(package);
    drop(texture);
    driver.owner.returned();
    assert_eq!(owner.live_bytes().unwrap(), 0);
}

#[test]
fn batches_task_b6_login_image_roles_bound_native_4k_and_tall_wide_scratch_without_changing_screen_contract(
) {
    for size in [[3840, 2160], [4096, 2304], [1, 4096], [4096, 1]] {
        let frame = FrameSize::new(size[0] as u32, size[1] as u32);
        assert_eq!(
            FrameLimits::LEGACY.check(frame).unwrap() as u64,
            pixel_bytes(size).unwrap()
        );
        assert!(
            super::super::screen_scaling::scratch_bytes(size, [4096, 1], &|| false).unwrap()
                <= super::super::screen_scaling::scratch_limit().unwrap()
        );
    }
    let pool = Arc::new(MemoryPool::default());
    let owner = local(&pool);
    let surfaces = [
        owner
            .try_claim(ImageRole::RenderSurface, LOGIN_BYTES)
            .unwrap(),
        owner
            .try_claim(ImageRole::RenderSurface, LOGIN_BYTES)
            .unwrap(),
    ];
    assert!(owner.try_claim(ImageRole::RenderSurface, 4).is_err());
    drop(surfaces);
    assert_eq!(
        crate::screen::core::frame_budget::VERSIONED_WORKING_IMAGES,
        24
    );
    assert_eq!(
        crate::screen::core::frame_budget::MAX_SESSION_WORKING_BYTES,
        864 * 1024 * 1024
    );
}
