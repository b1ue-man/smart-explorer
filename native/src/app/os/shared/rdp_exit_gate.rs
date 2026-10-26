//! Own the GUI's repeated RDP drain, including its thread's actual completion.
//! eframe 0.29.1 can veto Close only in update; on_exit is the final join fallback.
use std::{
    sync::{Arc, Mutex, PoisonError},
    thread::JoinHandle,
    time::{Duration, Instant},
};

use eframe::egui;

const RETRY: Duration = Duration::from_millis(250);

#[derive(Default)]
pub(super) struct RdpExitGate {
    started: bool,
    complete: bool,
    worker: Option<JoinHandle<()>>,
    error: Arc<Mutex<Option<String>>>,
    retry_at: Option<Instant>,
}

impl RdpExitGate {
    pub(super) fn error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Poll on subsequent frames. A slow daemon close never runs on the UI
    /// thread; the handle stays in App until its loop and destructors end.
    pub(super) fn poll(&mut self, ctx: &egui::Context) -> bool {
        if self.complete { return true; }
        self.started = true;
        if self.worker.as_ref().is_some_and(|worker| !worker.is_finished()) {
            ctx.request_repaint_after(RETRY);
            return false;
        }
        if let Some(worker) = self.worker.take() {
            if worker.join().is_ok() {
                self.complete = true;
                return true;
            }
            self.set_error("Der RDP-Abschluss wurde unerwartet unterbrochen und wird erneut versucht.".into());
        }
        if self.retry_at.is_some_and(|at| Instant::now() < at) {
            ctx.request_repaint_after(RETRY);
            return false;
        }
        // Atomic with start admission. With no ownership at all the existing
        // idle-close path remains synchronous and needs no temporary thread.
        if !crate::rdp::begin_owned_shutdown() {
            self.complete = true;
            return true;
        }
        let error = self.error.clone();
        let wake = ctx.clone();
        match std::thread::Builder::new().name("rdp-gui-exit".into()).spawn(move || {
            loop {
                match crate::rdp::shutdown_owned_sessions() {
                    Ok(()) => { wake.request_repaint(); return; }
                    Err(message) => {
                        let mut current = error.lock().unwrap_or_else(PoisonError::into_inner);
                        if current.as_ref() != Some(&message) {
                            *current = Some(message);
                            wake.request_repaint();
                        }
                    }
                }
                std::thread::sleep(RETRY);
            }
        }) {
            Ok(worker) => { self.worker = Some(worker); self.retry_at = None; }
            Err(error) => {
                self.set_error(format!("Der RDP-Abschluss konnte noch nicht starten: {error}. Der Start wird erneut versucht."));
                self.retry_at = Some(Instant::now() + RETRY);
            }
        }
        ctx.request_repaint_after(RETRY);
        false
    }

    fn set_error(&self, message: String) {
        *self.error.lock().unwrap_or_else(PoisonError::into_inner) = Some(message);
    }

    /// Only a completed drain can reopen admission after a cancelled exit or
    /// failed update/recovery preflight. Keep this owner while draining.
    pub(super) fn resume(&mut self, ctx: &egui::Context) -> Result<(), String> {
        if !self.started || !self.poll(ctx) { return Ok(()); }
        crate::rdp::resume_owned_sessions()?;
        *self = Self::default();
        Ok(())
    }

    pub(super) fn finish(&mut self) {
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                eprintln!("Smart Explorer: RDP-Abschluss wird nach einem unterbrochenen Worker erneut versucht.");
            }
        }
        let mut last_error = None;
        loop {
            match crate::rdp::shutdown_owned_sessions() {
                Ok(()) => { self.complete = true; return; }
                Err(error) => {
                    if last_error.as_ref() != Some(&error) {
                        eprintln!("Smart Explorer: RDP-Abschluss steht noch aus: {error}");
                        last_error = Some(error);
                    }
                    std::thread::sleep(RETRY);
                }
            }
        }
    }
}

impl Drop for RdpExitGate {
    fn drop(&mut self) {
        // JoinHandle::drop detaches. Neither gate reset nor App destruction may
        // detach a drain that still protects a child, lease or pending start.
        if self.started && !self.complete { self.finish(); }
    }
}
