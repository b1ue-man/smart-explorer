//! The automatic triggers of the session register (C3.3, C3.4): the timer
//! for idle and background time, the visibility and screen of the Android
//! app, and the system signals of the platform adapter (`vault/os/`).
use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError, Weak};
use std::time::{Duration, Instant};

use crate::vault::core::lock_policy::{
    self, LockReason, SignalSink, SignalSupport, SiteClock, SystemSignal,
};

use super::{LockRequest, SessionRegister};

/// The timer thread looks again at least this often.
const MAX_TIMER_WAIT: Duration = Duration::from_secs(30);
/// The timer never spins faster than this.
const MIN_TIMER_WAIT: Duration = Duration::from_millis(100);

impl SessionRegister {
    /// Visibility and screen of the Android app (`sys.hostState`, C3.4):
    /// screen off locks at once, the background after its minutes.
    pub(crate) fn set_host(&self, foreground: bool, screen_on: bool, now: Instant) {
        let screen_went_off = {
            let mut state = self.lock_state();
            let went_off = state.screen_on && !screen_on;
            state.screen_on = screen_on;
            if foreground {
                state.background_since = None;
            } else if state.background_since.is_none() {
                state.background_since = Some(now);
            }
            went_off
        };
        if screen_went_off {
            self.signal(SystemSignal::ScreenOff);
        }
        self.tick(now);
        self.changed.notify_all();
    }

    /// Locks the sites whose idle or background time is up at `now`;
    /// returns how long the timer may sleep.
    pub(crate) fn tick(&self, now: Instant) -> Duration {
        let platform = self.options.platform;
        let mut due = Vec::new();
        let mut wake = MAX_TIMER_WAIT;
        {
            let state = self.lock_state();
            let background = state
                .background_since
                .map(|since| now.saturating_duration_since(since));
            for (site, entry) in &state.sites {
                if entry.locking {
                    continue;
                }
                // An open file is no idle time (C3.3): its stream holds the
                // key cell of the session. The idle time starts again when
                // the last one closes.
                let idle = if Arc::strong_count(entry.session.keys()) > 1 {
                    entry.session.touch();
                    Duration::ZERO
                } else {
                    entry.session.idle_at(now)
                };
                let clock = SiteClock { idle, background };
                match lock_policy::due(&entry.settings, clock, platform) {
                    Some(reason) => due.push((site.clone(), reason)),
                    None => {
                        if let Some(left) = lock_policy::wake_in(&entry.settings, clock, platform) {
                            wake = wake.min(left.max(MIN_TIMER_WAIT));
                        }
                    }
                }
            }
        }
        for (site, reason) in due {
            // Forced: drives keep unsaved changes sealed (C3.3).
            let _ = self.lock(&site, LockRequest::new(reason, true));
        }
        wake
    }

    /// Which platform signals arrive (`Vault::Status`).
    pub(crate) fn signal_support(&self) -> SignalSupport {
        self.signals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|control| control.support())
            .unwrap_or_default()
    }

    /// Linux holds its sleep inhibitor only while a site locks on sleep.
    pub(super) fn rearm(&self) {
        let platform = self.options.platform;
        let armed = self
            .lock_state()
            .sites
            .values()
            .any(|entry| lock_policy::locks_on(&entry.settings, SystemSignal::Sleep, platform));
        if let Some(control) = self
            .signals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            control.set_armed(armed);
        }
    }

    /// Starts the timer thread and the platform signals once.
    pub(super) fn start_background(&self) {
        if !self.options.background || self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let sink: Weak<dyn SignalSink> = self.me.clone();
        let control = super::super::platform::start_signal_watch(sink);
        *self.signals.lock().unwrap_or_else(PoisonError::into_inner) = Some(control);
        let timer = self.me.clone();
        let spawned = std::thread::Builder::new()
            .name("vault-lock-timer".into())
            .spawn(move || loop {
                let Some(register) = timer.upgrade() else {
                    return;
                };
                let wait = register.tick(Instant::now());
                let state = register.lock_state();
                drop(
                    register
                        .changed
                        .wait_timeout(state, wait)
                        .unwrap_or_else(PoisonError::into_inner),
                );
            });
        if spawned.is_err() {
            // Without the timer only the idle and background locks are
            // missing; the next unlock tries again.
            self.started.store(false, Ordering::SeqCst);
        }
    }
}

impl SignalSink for SessionRegister {
    fn signal(&self, signal: SystemSignal) {
        let reason = match signal {
            SystemSignal::Sleep => LockReason::Sleep,
            SystemSignal::ScreenLocked => LockReason::ScreenLock,
            SystemSignal::ScreenOff => LockReason::ScreenOff,
            SystemSignal::Resume => return,
        };
        let platform = self.options.platform;
        self.lock_where(reason, |settings| {
            lock_policy::locks_on(settings, signal, platform)
        });
    }
}
