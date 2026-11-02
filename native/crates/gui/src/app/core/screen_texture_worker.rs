//! One conflating worker is reused across render generations; cancellation owns no pixels.
use super::screen_image_memory::CpuPixels;
use super::screen_presented::PresentedFrame;
use super::screen_render_upload::Generation;
use crate::screen::core::frame_memory::MemoryHold;
use eframe::egui;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
pub(super) struct Work {
    pub source: Arc<CpuPixels>,
    pub target: [usize; 2],
    pub seq: u64,
    pub frame: Option<PresentedFrame>,
    pub ctx: egui::Context,
    pub memory: MemoryHold,
    pub generation: Generation,
}
pub(super) struct Finished {
    pub source: [usize; 2],
    pub target: [usize; 2],
    pub seq: u64,
    pub frame: Option<PresentedFrame>,
    pub image: Option<Arc<CpuPixels>>,
    pub memory: MemoryHold,
    pub generation: Generation,
}
#[derive(Default)]
struct Slots {
    pending: Option<Work>,
    finished: Option<Finished>,
    stopped: bool,
    running: bool,
}
pub(super) struct Worker {
    shared: Arc<(Mutex<Slots>, Condvar)>,
    #[cfg(test)]
    gate: Arc<Mutex<Option<Arc<TestGate>>>>,
}
impl Worker {
    pub(super) fn new() -> Option<Self> {
        let shared = Arc::new((Mutex::new(Slots::default()), Condvar::new()));
        let thread = shared.clone();
        #[cfg(test)]
        let gate = Arc::new(Mutex::new(None::<Arc<TestGate>>));
        #[cfg(test)]
        let thread_gate = gate.clone();
        std::thread::Builder::new()
            .name("screen-scale".into())
            .spawn(move || loop {
                let (lock, ready) = &*thread;
                let work = {
                    let mut s = lock.lock().unwrap_or_else(PoisonError::into_inner);
                    while !s.stopped && s.pending.is_none() {
                        s = ready.wait(s).unwrap_or_else(PoisonError::into_inner);
                    }
                    if s.stopped {
                        return;
                    }
                    s.running = true;
                    s.pending.take()
                };
                let Some(work) = work else {
                    continue;
                };
                #[cfg(test)]
                let barrier = thread_gate.lock().unwrap().clone();
                #[cfg(test)]
                if let Some(gate) = &barrier {
                    if !gate.after_resample {
                        gate.enter();
                    }
                }
                let image =
                    super::screen_scaling::resample_admitted(&work.source, work.target, &|| {
                        !work.generation.active()
                            || work.frame.as_ref().is_some_and(|frame| !frame.active())
                    });
                #[cfg(test)]
                if let Some(gate) = &barrier {
                    if gate.after_resample {
                        gate.enter();
                    }
                }
                let mut s = lock.lock().unwrap_or_else(PoisonError::into_inner);
                s.running = false;
                if !s.stopped
                    && work.generation.active()
                    && work.frame.as_ref().is_none_or(|frame| frame.active())
                {
                    s.finished = Some(Finished {
                        source: work.source.size,
                        target: work.target,
                        seq: work.seq,
                        frame: work.frame.clone(),
                        image,
                        memory: work.memory.clone(),
                        generation: work.generation.clone(),
                    });
                } else {
                    drop(image);
                }
                let ctx = work.ctx.clone();
                drop(work);
                let stopped = s.stopped;
                drop(s);
                #[cfg(test)]
                if let Some(gate) = &barrier {
                    gate.exit();
                }
                super::screen_render_owner::notify_all();
                ctx.request_repaint();
                if stopped {
                    return;
                }
            })
            .ok()?;
        Some(Self {
            shared,
            #[cfg(test)]
            gate,
        })
    }
    pub(super) fn submit(&self, work: Work) {
        let mut s = self.shared.0.lock().unwrap_or_else(PoisonError::into_inner);
        s.pending = Some(work);
        self.shared.1.notify_one();
    }
    pub(super) fn take(&self) -> Option<Finished> {
        self.shared
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .finished
            .take()
    }
    pub(super) fn invalidate(&self) {
        let mut s = self.shared.0.lock().unwrap_or_else(PoisonError::into_inner);
        s.pending = None;
        s.finished = None;
        self.shared.1.notify_one();
        super::screen_render_owner::notify_all();
    }
    #[cfg(test)]
    pub(super) fn running(&self) -> bool {
        self.shared.0.lock().unwrap().running
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let mut s = self.shared.0.lock().unwrap_or_else(PoisonError::into_inner);
        s.stopped = true;
        s.pending = None;
        s.finished = None;
        self.shared.1.notify_one();
        super::screen_render_owner::notify_all();
    }
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct TestGate {
    after_resample: bool,
    state: Mutex<(bool, bool, bool)>,
    ready: Condvar,
}
#[cfg(test)]
impl TestGate {
    pub(super) fn after_resample() -> Self {
        Self {
            after_resample: true,
            ..Self::default()
        }
    }
    fn enter(&self) {
        let mut s = self.state.lock().unwrap();
        s.0 = true;
        self.ready.notify_all();
        while !s.1 {
            s = self.ready.wait(s).unwrap();
        }
    }
    fn exit(&self) {
        self.state.lock().unwrap().2 = true;
        self.ready.notify_all();
    }
    pub(super) fn wait_entered(&self) {
        let mut s = self.state.lock().unwrap();
        while !s.0 {
            s = self.ready.wait(s).unwrap();
        }
    }
    pub(super) fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.ready.notify_all();
    }
    pub(super) fn wait_exited(&self) {
        let mut s = self.state.lock().unwrap();
        while !s.2 {
            s = self.ready.wait(s).unwrap();
        }
    }
}
#[cfg(test)]
impl Worker {
    pub(super) fn set_gate(&self, gate: Arc<TestGate>) {
        *self.gate.lock().unwrap() = Some(gate);
    }
}
