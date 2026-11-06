//! Restarts the paused, lost and refused device feeds as soon as this device leaves
//! low-power operation (A4 of plan 2026-11-06-akku-rechte): the app became
//! visible. `observe` runs only after events and reloads (on Android at
//! least once a minute), which is too late for a person who just opened
//! the app. The thread starts with the first paused feed, so a desktop,
//! which never enters low-power operation, never runs it. It waits on the
//! power hub without any timer; the hub wakes it on every power change and
//! with every connection probe.
use std::sync::{Mutex, PoisonError};

pub(super) fn follow_visibility() {
    static STARTED: Mutex<bool> = Mutex::new(false);
    let mut started = STARTED.lock().unwrap_or_else(PoisonError::into_inner);
    if *started {
        return;
    }
    match std::thread::Builder::new()
        .name("share-peer-feed-power".into())
        .spawn(run)
    {
        Ok(_) => *started = true,
        // The feeds then start again with the next `observe`, and the next
        // pause tries to start the thread again.
        Err(error) => eprintln!("Gerätefeed: Wächter der Sichtbarkeit startet nicht: {error}"),
    }
}

fn run() {
    let (sender, wake) = crossbeam_channel::bounded(1);
    let subscription = crate::share::power::global().subscribe(sender);
    // A change between the pause that started this thread and the
    // subscription woke nobody.
    if !crate::share::power::low_power() {
        super::resume();
    }
    while wake.recv().is_ok() {
        // The hub hands every subscriber a share of a connection probe; this
        // one is no signal worker and answers at once with an empty answer
        // (the workers' answers decide the outcome), as `keep_awake` does.
        drop(subscription.take_probe());
        if !crate::share::power::low_power() {
            super::resume();
        }
    }
}
