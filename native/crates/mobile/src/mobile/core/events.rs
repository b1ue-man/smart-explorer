//! Event queue behind `poll_events` (api.md §3): plain events in arrival
//! order plus bundled task snapshots from the task register.
use super::tasks::TaskTable;
use serde_json::Value;
use std::collections::VecDeque;
use std::time::Instant;

/// Most events one `pollEvents` answer carries.
pub(crate) const MAX_POLL_EVENTS: usize = 256;
/// Plain events kept while nobody polls; the oldest are dropped first.
const MAX_QUEUED_EVENTS: usize = 2048;

/// Shared state of the event hub, guarded by one mutex.
pub(crate) struct HubState {
    pub(crate) tasks: TaskTable,
    events: VecDeque<Value>,
    dropped: u64,
}

impl HubState {
    pub(crate) fn new(run_tag: i64) -> Self {
        Self {
            tasks: TaskTable::new(run_tag),
            events: VecDeque::new(),
            dropped: 0,
        }
    }

    pub(crate) fn push_event(&mut self, event: Value) {
        if self.events.len() >= MAX_QUEUED_EVENTS {
            self.events.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.events.push_back(event);
    }

    /// Events that can be delivered now (at most `max`), and when the next
    /// held-back task snapshot becomes due.
    pub(crate) fn take_ready(&mut self, now: Instant, max: usize) -> (Vec<Value>, Option<Instant>) {
        let mut ready = Vec::new();
        if self.dropped > 0 && max > 0 {
            ready.push(serde_json::json!({
                "type": "error",
                "action": "Ereignisse",
                "message": format!("{} Ereignisse verworfen (niemand hat sie abgeholt)", self.dropped),
            }));
            self.dropped = 0;
        }
        while ready.len() < max {
            match self.events.pop_front() {
                Some(event) => ready.push(event),
                None => break,
            }
        }
        let room = max.saturating_sub(ready.len());
        let (tasks, next_due) = self.tasks.take_due(now, room);
        ready.extend(tasks);
        let next_due = if self.events.is_empty() {
            next_due
        } else {
            Some(now)
        };
        (ready, next_due)
    }
}
