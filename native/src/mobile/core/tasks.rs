//! Task register (api.md §3): every long-running call is a task with an id,
//! progress, errors and a result. Snapshots are emitted as `task` events at
//! most `EMIT_INTERVAL` apart per task; the terminal snapshot is never held back.
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// At most four progress snapshots per second and task.
pub(crate) const EMIT_INTERVAL: Duration = Duration::from_millis(250);
/// Errors kept per task; further ones are only counted.
const MAX_TASK_ERRORS: usize = 100;
/// Finished tasks kept until `task.clear`; older ones are dropped first.
const MAX_FINISHED_TASKS: usize = 200;
const RATE_SAMPLE: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskState {
    Queued,
    Running,
    Done,
    Failed,
    Canceled,
}

impl TaskState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            TaskState::Queued => "queued",
            TaskState::Running => "running",
            TaskState::Done => "done",
            TaskState::Failed => "failed",
            TaskState::Canceled => "canceled",
        }
    }

    pub(crate) fn is_finished(self) -> bool {
        matches!(
            self,
            TaskState::Done | TaskState::Failed | TaskState::Canceled
        )
    }
}

pub(crate) struct TaskRecord {
    pub(crate) id: String,
    pub(crate) kind: String,
    title: String,
    pub(crate) state: TaskState,
    done_bytes: u64,
    total_bytes: u64,
    done_items: u64,
    total_items: u64,
    rate_bps: u64,
    message: Option<String>,
    errors: Vec<(String, String)>,
    errors_total: u64,
    result: Option<Value>,
    started_ms: i64,
    finished_ms: Option<i64>,
    pub(crate) cancel: Arc<AtomicBool>,
    dirty: bool,
    last_emit: Option<Instant>,
    rate_sample: Option<(Instant, u64)>,
}

impl TaskRecord {
    pub(crate) fn set_running(&mut self) {
        if self.state == TaskState::Queued {
            self.state = TaskState::Running;
            self.dirty = true;
        }
    }

    pub(crate) fn progress(&mut self, counts: (u64, u64, u64, u64), now: Instant) {
        let (done_bytes, total_bytes, done_items, total_items) = counts;
        self.update_rate(done_bytes, now);
        self.done_bytes = done_bytes;
        self.total_bytes = total_bytes;
        self.done_items = done_items;
        self.total_items = total_items;
        self.dirty = true;
    }

    fn update_rate(&mut self, bytes: u64, now: Instant) {
        match self.rate_sample {
            Some((at, sampled)) if bytes >= sampled => {
                let elapsed = now.saturating_duration_since(at);
                if elapsed >= RATE_SAMPLE {
                    let millis = elapsed.as_millis().max(1) as u64;
                    let current = (bytes - sampled).saturating_mul(1000) / millis;
                    self.rate_bps = if self.rate_bps == 0 {
                        current
                    } else {
                        (self.rate_bps.saturating_mul(2)).saturating_add(current) / 3
                    };
                    self.rate_sample = Some((now, bytes));
                }
            }
            _ => self.rate_sample = Some((now, bytes)),
        }
    }

    pub(crate) fn set_message(&mut self, text: &str) {
        self.message = (!text.is_empty()).then(|| text.to_string());
        self.dirty = true;
    }

    pub(crate) fn push_error(&mut self, path: &str, message: &str) {
        self.errors_total = self.errors_total.saturating_add(1);
        if self.errors.len() < MAX_TASK_ERRORS {
            self.errors.push((path.to_string(), message.to_string()));
        }
        self.dirty = true;
    }

    /// Enters a terminal state; later calls are ignored.
    pub(crate) fn finish(
        &mut self,
        state: TaskState,
        message: Option<String>,
        result: Option<Value>,
        now_ms: i64,
    ) {
        if self.state.is_finished() || !state.is_finished() {
            return;
        }
        self.state = state;
        if message.is_some() {
            self.message = message;
        }
        self.result = result;
        self.finished_ms = Some(now_ms);
        self.rate_bps = 0;
        self.dirty = true;
    }

    pub(crate) fn snapshot(&self) -> Value {
        let mut errors: Vec<Value> = self
            .errors
            .iter()
            .map(|(path, message)| json!({ "path": path, "message": message }))
            .collect();
        let hidden = self.errors_total.saturating_sub(self.errors.len() as u64);
        if hidden > 0 {
            errors.push(json!({ "path": "", "message": format!("… und {hidden} weitere Fehler") }));
        }
        json!({
            "id": self.id,
            "kind": self.kind,
            "title": self.title,
            "state": self.state.as_str(),
            "doneBytes": self.done_bytes,
            "totalBytes": self.total_bytes,
            "doneItems": self.done_items,
            "totalItems": self.total_items,
            "rateBps": self.rate_bps,
            "message": self.message,
            "errors": errors,
            "result": self.result,
            "startedMs": self.started_ms,
            "finishedMs": self.finished_ms,
        })
    }
}

/// All tasks of this process run, oldest first.
pub(crate) struct TaskTable {
    records: Vec<TaskRecord>,
    prefix: String,
    next: u64,
}

impl TaskTable {
    /// `run_tag` keeps ids unique across process restarts.
    pub(crate) fn new(run_tag: i64) -> Self {
        Self {
            records: Vec::new(),
            prefix: format!("t{:x}", run_tag.max(0)),
            next: 0,
        }
    }

    pub(crate) fn create(
        &mut self,
        kind: &str,
        title: String,
        cancel: Arc<AtomicBool>,
        now_ms: i64,
    ) -> String {
        self.next = self.next.saturating_add(1);
        let id = format!("{}-{}", self.prefix, self.next);
        self.records.push(TaskRecord {
            id: id.clone(),
            kind: kind.to_string(),
            title,
            state: TaskState::Queued,
            done_bytes: 0,
            total_bytes: 0,
            done_items: 0,
            total_items: 0,
            rate_bps: 0,
            message: None,
            errors: Vec::new(),
            errors_total: 0,
            result: None,
            started_ms: now_ms,
            finished_ms: None,
            cancel,
            dirty: true,
            last_emit: None,
            rate_sample: None,
        });
        self.trim_finished();
        id
    }

    pub(crate) fn get(&self, id: &str) -> Option<&TaskRecord> {
        self.records.iter().find(|record| record.id == id)
    }

    pub(crate) fn get_mut(&mut self, id: &str) -> Option<&mut TaskRecord> {
        self.records.iter_mut().find(|record| record.id == id)
    }

    pub(crate) fn snapshots(&self) -> Vec<Value> {
        self.records.iter().map(TaskRecord::snapshot).collect()
    }

    /// Removes finished tasks whose terminal snapshot was already delivered.
    pub(crate) fn clear_finished(&mut self) {
        self.records
            .retain(|record| !record.state.is_finished() || record.dirty);
    }

    /// Requests cancellation; false when the id is unknown.
    pub(crate) fn cancel(&self, id: &str) -> bool {
        match self.get(id) {
            Some(record) => {
                record.cancel.store(true, Ordering::Release);
                true
            }
            None => false,
        }
    }

    pub(crate) fn cancel_all(&self, kind: Option<&str>) {
        for record in &self.records {
            if !record.state.is_finished() && kind.is_none_or(|kind| record.kind == kind) {
                record.cancel.store(true, Ordering::Release);
            }
        }
    }

    /// Snapshots that are due now and the time the next pending one is due.
    pub(crate) fn take_due(&mut self, now: Instant, max: usize) -> (Vec<Value>, Option<Instant>) {
        let mut due = Vec::new();
        let mut next: Option<Instant> = None;
        for record in self.records.iter_mut().filter(|record| record.dirty) {
            let ready_at = match record.last_emit {
                Some(last) if !record.state.is_finished() => last + EMIT_INTERVAL,
                _ => now,
            };
            if ready_at <= now && due.len() < max {
                due.push(json!({ "type": "task", "task": record.snapshot() }));
                record.dirty = false;
                record.last_emit = Some(now);
            } else {
                let at = ready_at.max(now);
                next = Some(next.map_or(at, |current| current.min(at)));
            }
        }
        (due, next)
    }

    fn trim_finished(&mut self) {
        let finished = self
            .records
            .iter()
            .filter(|record| record.state.is_finished())
            .count();
        let mut excess = finished.saturating_sub(MAX_FINISHED_TASKS);
        if excess == 0 {
            return;
        }
        self.records.retain(|record| {
            if excess > 0 && record.state.is_finished() && !record.dirty {
                excess -= 1;
                false
            } else {
                true
            }
        });
    }
}
