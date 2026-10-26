//! Controlled output, UI and worker fixtures. These cases do not claim a live GL context.
use super::super::super::{
    screen_image_memory::{owner_limits, CpuPixels, ImageAccess},
    screen_render_cpu::{finish_glow_draw, finish_glow_upload},
    screen_render_glow::{native_texture, GlowImage, GlowUpload},
};
use super::*;
use crate::screen::core::{
    frame_memory::CapacityCeiling,
    image_admission::{ImageAdmission, ImageRole, LocalImagePolicy},
};
use eframe::egui_glow::{glow, GlOperationError};

fn local_owner() -> Arc<ImageAdmission> {
    ImageAdmission::in_test_pool(
        Arc::new(MemoryPool::default()),
        LocalImagePolicy {
            facts: None,
            ceiling: CapacityCeiling::bounded(MAX_WORKING_BYTES),
        },
        owner_limits().unwrap(),
    )
}
fn packet(
    slot: &RenderSlot,
    owner: &Arc<ImageAdmission>,
    frame: PresentedFrame,
    seq: u64,
) -> Package {
    Package::Cpu(super::super::super::screen_render_upload::PreparedImage {
        image: CpuPixels::convert(&image(), ImageAccess::Local(owner.clone())).unwrap(),
        memory: None,
        frame: Some(frame),
        generation: slot.generation(),
        seq,
    })
}
fn successful_upload(slot: &RenderSlot, package: Package) {
    let mut s = lock(&slot.state);
    let claims = s.claim_upload(&package).unwrap();
    let previous = s.resident.take();
    assert!(finish_glow_upload(
        &mut s,
        package,
        claims,
        previous,
        GlowUpload {
            image: GlowImage {
                id: egui::TextureId::Managed(1),
                size: [2, 2]
            },
            result: Ok(()),
            drain: Drain::default(),
        },
        |_| Drain::default()
    ));
}

#[test]
fn batches_task_b6_glow_zero_texture_name_returns_error_without_a_panic_hook() {
    unsafe extern "system" fn zero(_: i32, out: *mut u32) {
        unsafe {
            *out = 0;
        }
    }
    unsafe extern "system" fn created(_: i32, out: *mut u32) {
        unsafe {
            *out = 7;
        }
    }
    assert!(native_texture(zero).is_err());
    assert_eq!(native_texture(created).unwrap().0.get(), 7);
}

#[test]
fn batches_task_b6_glow_failed_overwrite_has_no_resident_and_keeps_both_sources_until_drain() {
    let ctx = egui::Context::default();
    let driver = ControlledDriver::install(&ctx);
    let owner = local_owner();
    let slot = Arc::new(RenderSlot::default());
    driver.owner.attach(&slot);
    let first = stamp(1);
    successful_upload(&slot, packet(&slot, &owner, first.clone(), 1));
    driver.complete_upload(&slot);
    let old = match &lock(&slot.state).resident.as_ref().unwrap().package {
        Package::Cpu(p) => Arc::downgrade(&p.image),
        _ => unreachable!(),
    };
    let mut second = first;
    second.frame_id = 2;
    let package = packet(&slot, &owner, second.clone(), 2);
    let mut s = lock(&slot.state);
    let claims = s.claim_upload(&package).unwrap();
    let previous = s.resident.take();
    assert!(!finish_glow_upload(
        &mut s,
        package,
        claims,
        previous,
        GlowUpload {
            image: GlowImage {
                id: egui::TextureId::Managed(1),
                size: [2, 2]
            },
            result: Err(GlOperationError::from_gl_code(glow::INVALID_OPERATION)),
            drain: Drain::default(),
        },
        |_| Drain::default()
    ));
    assert!(s.resident.is_none());
    assert!(s.retiring.as_ref().unwrap().superseded.is_some());
    drop(s);
    assert_eq!(owner.live_count(ImageRole::ColorSource).unwrap(), 2);
    assert_eq!(owner.live_count(ImageRole::RenderSurface).unwrap(), 1);
    assert_eq!(owner.live_count(ImageRole::UploadStaging).unwrap(), 1);
    assert!(!driver.draw(
        &slot,
        second,
        Arc::new(|| panic!("failed upload must not produce a receipt"))
    ));
    driver.complete_retirement(&slot);
    assert!(old.upgrade().is_some());
    driver.complete_upload(&slot);
    assert!(old.upgrade().is_none());
    assert_eq!(owner.live_count(ImageRole::RenderSurface).unwrap(), 0);
    assert_eq!(owner.live_count(ImageRole::UploadStaging).unwrap(), 0);
    assert_eq!(
        owner.live_count(ImageRole::ColorSource).unwrap(),
        1,
        "only the retry candidate remains"
    );
    driver.owner.returned();
    assert_eq!(owner.live_bytes().unwrap(), 0);
}

#[test]
fn batches_task_b6_glow_successful_upload_followed_by_failed_draw_never_quittiert_new_stamp() {
    let ctx = egui::Context::default();
    let driver = ControlledDriver::install(&ctx);
    let owner = local_owner();
    let slot = Arc::new(RenderSlot::default());
    driver.owner.attach(&slot);
    let first = stamp(1);
    successful_upload(&slot, packet(&slot, &owner, first.clone(), 1));
    driver.complete_upload(&slot);
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let receipt: super::super::super::screen_presented::Presented = Arc::new(move || {
        count.fetch_add(1, Ordering::SeqCst);
    });
    assert!(driver.draw(&slot, first.clone(), receipt.clone()));
    let mut second = first;
    second.frame_id = 2;
    successful_upload(&slot, packet(&slot, &owner, second.clone(), 2));
    let result = finish_glow_draw(
        &mut lock(&slot.state),
        Err(GlOperationError::from_gl_code(
            glow::INVALID_FRAMEBUFFER_OPERATION,
        )),
        |id| {
            assert_eq!(id, egui::TextureId::Managed(1));
            Drain::default()
        },
    );
    assert!(!result);
    assert!(!driver.draw(&slot, second, receipt));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(slot.frame().is_none());
    assert!(slot.id().is_none());
    assert_eq!(owner.live_count(ImageRole::RenderSurface).unwrap(), 1);
    driver.complete_retirement(&slot);
    assert_eq!(owner.live_count(ImageRole::RenderSurface).unwrap(), 1);
    driver.complete_upload(&slot);
    assert_eq!(owner.live_count(ImageRole::RenderSurface).unwrap(), 0);
    driver.owner.returned();
    assert_eq!(owner.live_bytes().unwrap(), 0);
}

#[test]
fn batches_task_b6_glow_oom_attempt_keeps_real_claims_until_native_instance_return() {
    let ctx = egui::Context::default();
    let driver = ControlledDriver::install(&ctx);
    let owner = local_owner();
    let slot = Arc::new(RenderSlot::default());
    driver.owner.attach(&slot);
    let frame = stamp(1);
    let package = packet(&slot, &owner, frame.clone(), 1);
    let mut s = lock(&slot.state);
    let claims = s.claim_upload(&package).unwrap();
    let error = GlOperationError::from_gl_code(glow::OUT_OF_MEMORY);
    assert!(error.state_unreliable());
    assert!(!finish_glow_upload(
        &mut s,
        package,
        claims,
        None,
        GlowUpload {
            image: GlowImage {
                id: egui::TextureId::Managed(1),
                size: [2, 2]
            },
            result: Err(error),
            drain: Drain::default(),
        },
        |_| Drain::default()
    ));
    drop(s);
    assert!(!driver.draw(
        &slot,
        frame,
        Arc::new(|| panic!("OOM is not a successful presentation"))
    ));
    driver.owner.shutdown(&ctx, None);
    assert_eq!(owner.live_count(ImageRole::RenderSurface).unwrap(), 1);
    assert_eq!(owner.live_count(ImageRole::UploadStaging).unwrap(), 1);
    assert_eq!(owner.live_count(ImageRole::ColorSource).unwrap(), 1);
    // Concrete return is the explicit event used for an unreliable GL instance.
    driver.owner.returned();
    assert_eq!(owner.live_bytes().unwrap(), 0);
}
#[test]
fn batches_task_b6_renderer_discarded_output_does_not_free_a_resident_glow_role() {
    let ctx = egui::Context::default();
    let driver = ControlledDriver::install(&ctx);
    let pool = Arc::new(MemoryPool::default());
    let memory = Arc::new(pool.reserve(None, MAX_WORKING_BYTES).unwrap());
    let mut t = texture(&ctx, Some(memory.clone()));
    drop(memory);
    driver.upload();
    driver.complete_upload(t.slot());
    let slot = t.slot().clone();
    let discarded = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            assert!(t.paint(ui.painter(), ui.max_rect(), |_| None));
        });
    });
    drop(discarded);
    assert!(slot.frame().is_some());
    assert!(pool.reserve(None, 1024 * 1024).is_err());
    t.reset();
    driver.owner.after_update(&ctx);
    drop(t);
    assert!(pool.reserve(None, 1024 * 1024).is_err());
    driver.complete_retirement(&slot);
    driver.owner.after_update(&ctx);
    assert!(pool.reserve(None, MAX_WORKING_BYTES).is_ok());
}
#[test]
fn batches_task_b6_renderer_scaler_retirement_waits_for_actual_running_work_exit() {
    use super::super::super::screen_texture_worker::{TestGate, Work, Worker};
    for after_resample in [false, true] {
        let ctx = egui::Context::default();
        let pool = Arc::new(MemoryPool::default());
        let memory = Arc::new(pool.reserve(None, MAX_WORKING_BYTES).unwrap());
        let generation = Generation::default();
        let gate = Arc::new(if after_resample {
            TestGate::after_resample()
        } else {
            TestGate::default()
        });
        let worker = Worker::new().unwrap();
        worker.set_gate(gate.clone());
        worker.submit(Work {
            source: super::super::super::screen_image_memory::CpuPixels::convert(
                &BgraImage::new(FrameSize::new(32, 32)).unwrap(),
                super::super::super::screen_image_memory::ImageAccess::Contract(Some(
                    memory.clone(),
                )),
            )
            .unwrap(),
            target: [64, 64],
            seq: 1,
            frame: Some(stamp(1)),
            ctx,
            memory: Some(memory.clone()),
            generation: generation.clone(),
        });
        drop(memory);
        gate.wait_entered();
        assert!(worker.running());
        generation.revoke();
        worker.invalidate();
        assert!(pool.reserve(None, 1024 * 1024).is_err());
        drop(worker);
        assert!(pool.reserve(None, 1024 * 1024).is_err());
        gate.release();
        gate.wait_exited();
        assert!(pool.reserve(None, MAX_WORKING_BYTES).is_ok());
    }
}
