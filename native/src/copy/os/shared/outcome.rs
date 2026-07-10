use super::{CopyMsg, CopyProgress};
use crossbeam_channel::Sender;

const COPY_ERROR_LIMIT: usize = 100;

#[derive(Default)]
pub(super) struct CopyErrorLog {
    total: u64,
    items: Vec<(String, String)>,
}

impl CopyErrorLog {
    pub(super) fn record(&mut self, path: String, detail: String) {
        self.total = self.total.saturating_add(1);
        if self.items.len() < COPY_ERROR_LIMIT {
            self.items.push((path, detail));
        }
    }

    pub(super) fn total(&self) -> u64 {
        self.total
    }

    pub(super) fn into_items(self) -> Vec<(String, String)> {
        self.items
    }
}

pub(super) fn send_copy_canceled(tx: &Sender<CopyMsg>) {
    send_terminal(tx, true, 0, Vec::new());
}

pub(super) fn send_collection_failure(
    tx: &Sender<CopyMsg>,
    outcome: crate::scanner::CollectOutcome,
) {
    let mut errors = CopyErrorLog::default();
    for issue in outcome.issues {
        errors.record(issue.path, issue.detail);
    }
    for _ in 0..outcome.suppressed_issues {
        errors.total = errors.total.saturating_add(1);
    }
    send_terminal(tx, false, errors.total(), errors.into_items());
}

fn send_terminal(
    tx: &Sender<CopyMsg>,
    canceled: bool,
    error_count: u64,
    errors: Vec<(String, String)>,
) {
    let _ = tx.send(CopyMsg::Done {
        progress: CopyProgress {
            files_done: 0,
            files_total: 0,
            bytes_done: 0,
            bytes_total: 0,
            elapsed_ms: 0,
            errors: error_count,
            canceled,
            done: true,
        },
        errors,
    });
}
