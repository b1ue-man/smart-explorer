//! Test support: the broker's answer a test stands in for (`broker_view`),
//! for a process that has no login service (two-node test of the login
//! target, `login_ziel_knoten_task_tests.rs`). `None` asks the real broker.
use std::sync::{Mutex, PoisonError};

use crate::login::core::self_grants::SelfReply;

static STAND_IN: Mutex<Option<SelfReply>> = Mutex::new(None);

/// Sets (or with `None` removes) the answer `broker_view` gives.
pub fn set(reply: Option<SelfReply>) {
    *STAND_IN.lock().unwrap_or_else(PoisonError::into_inner) = reply;
}

pub(super) fn reply() -> Option<SelfReply> {
    STAND_IN
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}
