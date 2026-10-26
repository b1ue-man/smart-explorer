//! Presentation-bound commands and cancellation of the existing viewer owner.
use super::{invalid, refresh, Command, Shared, State, ViewerSession};
use crate::login::FrameToken;
use crate::screen::core::input_model::{encode_batch, InputEvent};
use std::sync::atomic::Ordering;
use std::time::Duration;

impl ViewerSession {
    pub fn presented(&self, token: &FrameToken, epoch: u64) -> bool {
        let Ok(mut state) = self.shared.state.lock() else {
            return false;
        };
        let accepted = state.guard.presented(token, epoch);
        refresh(&mut state);
        accepted
    }
    pub fn invalidate_presentation(&self) {
        self.invalidate_presentation_at(None);
    }
    /// A late renderer failure may revoke only the epoch that owned its pixels.
    pub(crate) fn renderer_failed(&self, epoch: u64) {
        self.invalidate_presentation_at(Some(epoch));
    }
    fn invalidate_presentation_at(&self, epoch: Option<u64>) {
        let mut state = match self.shared.state.lock() {
            Ok(state) => state,
            Err(error) => {
                drop(error);
                self.close();
                return;
            }
        };
        if epoch.is_some_and(|epoch| epoch != state.guard.epoch) {
            return;
        }
        self.shared.release.store(true, Ordering::SeqCst);
        state.guard.invalidate();
        let interrupt = must_interrupt(&self.shared, &state);
        refresh(&mut state);
        drop(state);
        if interrupt {
            self.close();
        }
    }
    pub fn focus(&self, focused: bool, control: bool) {
        let mut interrupt = false;
        if let Ok(mut state) = self.shared.state.lock() {
            if state.guard.focused != focused || state.guard.control != control {
                state.guard.invalidate();
            }
            if (state.guard.focused && !focused) || (state.guard.control && !control) {
                self.shared.release.store(true, Ordering::SeqCst);
                interrupt = must_interrupt(&self.shared, &state);
            }
            state.guard.focused = focused;
            state.guard.control = control;
            refresh(&mut state);
        }
        if interrupt {
            self.close();
        }
    }
    pub fn input(
        &self,
        token: FrameToken,
        epoch: u64,
        events: Vec<InputEvent>,
    ) -> Result<(), String> {
        encode_batch(&events).map_err(invalid)?;
        if events
            .iter()
            .any(|event| matches!(event, InputEvent::Action { .. }))
        {
            return Err("Android actions are unavailable at login".into());
        }
        let state = self
            .shared
            .state
            .lock()
            .map_err(|_| "login viewer state is unavailable")?;
        let allowed = state.guard.epoch == epoch && state.guard.input_ready(&token);
        drop(state);
        if !allowed {
            return Err("login input requires the focused, actually drawn current frame".into());
        }
        self.queue(Command::Input(token, epoch, events))
    }
    pub fn secure_attention(&self, token: FrameToken, epoch: u64) -> Result<(), String> {
        let state = self
            .shared
            .state
            .lock()
            .map_err(|_| "login viewer state is unavailable")?;
        if state.guard.epoch != epoch
            || !state.guard.action_ready(&token)
            || !state
                .snapshot
                .view
                .as_ref()
                .and_then(|view| view.access.as_ref())
                .is_some_and(|access| {
                    access.rights.sas && access.rights.screen_control && access.rights.login
                })
        {
            return Err("secure attention requires the current drawn frame and explicit administrative SAS/control rights".into());
        }
        drop(state);
        self.queue(Command::Sas(token, epoch))
    }
    fn queue(&self, command: Command) -> Result<(), String> {
        if self.shared.stopped.load(Ordering::SeqCst) {
            return Err("login viewer ended".into());
        }
        if self.commands.try_send(command).is_err() {
            self.close();
            return Err("login input queue is full or closed; session ended without retry".into());
        }
        Ok(())
    }
    pub fn release(&self) {
        self.shared.release.store(true, Ordering::SeqCst);
        let mut interrupt = false;
        if let Ok(mut state) = self.shared.state.lock() {
            state.guard.focused = false;
            state.guard.control = false;
            state.guard.invalidate();
            interrupt = must_interrupt(&self.shared, &state);
            refresh(&mut state);
        }
        if interrupt {
            self.close();
        }
    }
    pub fn close(&self) {
        if self.shared.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Ok(mut state) = self.shared.state.lock() {
            state.guard.close();
            state.frame = None;
            state.snapshot.state = "stopping".into();
            refresh(&mut state);
            if let Some(close) = &state.close {
                close.close();
            }
        }
        self.shared.changed.notify_all();
    }
    /// Legacy JNI copying has no admitted destination owner. L7 replaces
    /// this entrypoint with the bitmap-ticket transfer of a whole ViewerFrame.
    /// No raw copy may escape through the old DirectBuffer/IntArray path.
    pub fn pull(&self, _dst: &mut [u8], _wait: Duration) -> i32 {
        if self.shared.stopped.load(Ordering::SeqCst) {
            -2
        } else {
            -4
        }
    }
}

fn must_interrupt(shared: &Shared, state: &State) -> bool {
    shared.input_flight.load(Ordering::SeqCst)
        || (shared.operating.load(Ordering::SeqCst) && !state.pressed.is_empty())
}
pub(super) fn command_allowed(state: &State, command: &Command) -> bool {
    let rights = state
        .snapshot
        .view
        .as_ref()
        .and_then(|view| view.access.as_ref())
        .map(|access| &access.rights);
    match command {
        Command::Input(token, epoch, _) => {
            state.guard.epoch == *epoch
                && state.guard.input_ready(token)
                && rights.is_some_and(|rights| {
                    rights.screen_control && rights.screen_view && rights.login
                })
        }
        Command::Sas(token, epoch) => {
            state.guard.epoch == *epoch
                && state.guard.action_ready(token)
                && rights.is_some_and(|rights| rights.sas && rights.screen_control && rights.login)
        }
    }
}
