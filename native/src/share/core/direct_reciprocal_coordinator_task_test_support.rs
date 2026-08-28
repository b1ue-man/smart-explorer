use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};

use super::{DirectReciprocalCoordinator, Shared, State};

impl DirectReciprocalCoordinator {
    pub(crate) fn detached_for_task_test(generation: u64) -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    generation,
                    stopped: false,
                    next_epoch: 1,
                    tasks: HashMap::new(),
                }),
                wake: Condvar::new(),
            }),
            worker: None,
        }
    }

    pub(crate) fn generation_for_task_test(&self) -> u64 {
        self.shared.state.lock().unwrap().generation
    }

    pub(crate) fn task_count_for_task_test(&self) -> usize {
        self.shared.state.lock().unwrap().tasks.len()
    }
}
