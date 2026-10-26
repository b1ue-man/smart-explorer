//! Controlled output, UI and worker fixtures. These cases do not claim a live GL context.
use super::*;
#[test]
fn batches_task_b6_renderer_discarded_output_does_not_free_a_resident_glow_role() {
    let ctx=egui::Context::default();let driver=ControlledDriver::install(&ctx);let pool=Arc::new(MemoryPool::default());
    let memory=Arc::new(pool.reserve(None,MAX_WORKING_BYTES).unwrap());let mut t=texture(&ctx,Some(memory.clone()));drop(memory);
    driver.upload();driver.complete_upload(t.slot());let slot=t.slot().clone();
    let discarded=ctx.run(egui::RawInput::default(),|ctx| {egui::CentralPanel::default().show(ctx,|ui| {assert!(t.paint(ui.painter(),ui.max_rect(),|_|None));});});
    drop(discarded);assert!(slot.frame().is_some());assert!(pool.reserve(None,1024*1024).is_err());
    t.reset();driver.owner.after_update(&ctx);drop(t);assert!(pool.reserve(None,1024*1024).is_err());
    driver.complete_retirement(&slot);driver.owner.after_update(&ctx);assert!(pool.reserve(None,MAX_WORKING_BYTES).is_ok());
}
#[test]
fn batches_task_b6_renderer_scaler_retirement_waits_for_actual_running_work_exit() {
    use super::super::super::screen_texture_worker::{Worker,Work,TestGate};
    for after_resample in [false,true] {
    let ctx=egui::Context::default();let pool=Arc::new(MemoryPool::default());let memory=Arc::new(pool.reserve(None,MAX_WORKING_BYTES).unwrap());
    let generation=Generation::default();let gate=Arc::new(if after_resample {TestGate::after_resample()} else {TestGate::default()});let worker=Worker::new().unwrap();worker.set_gate(gate.clone());
    worker.submit(Work {source:Arc::new(egui::ColorImage::new([32,32],egui::Color32::WHITE)),target:[64,64],seq:1,frame:Some(stamp(1)),ctx,memory:Some(memory.clone()),generation:generation.clone()});
    drop(memory);gate.wait_entered();assert!(worker.running());generation.revoke();worker.invalidate();
    assert!(pool.reserve(None,1024*1024).is_err());drop(worker);assert!(pool.reserve(None,1024*1024).is_err());
    gate.release();gate.wait_exited();assert!(pool.reserve(None,MAX_WORKING_BYTES).is_ok());
    }
}
