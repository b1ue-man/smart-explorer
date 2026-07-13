use std::sync::{Arc, Mutex};

use super::state::{cleanup, State};

/// Removes every server-side registration owned by one transport, regardless
/// of which error or disconnect path ends the handler.
pub(super) struct RegistrationGuard {
    id: u64,
    state: Arc<Mutex<State>>,
}

impl RegistrationGuard {
    pub(super) fn new(id: u64, state: &Arc<Mutex<State>>) -> Self {
        Self {
            id,
            state: Arc::clone(state),
        }
    }
}

impl Drop for RegistrationGuard {
    fn drop(&mut self) {
        cleanup(self.id, &self.state);
    }
}
