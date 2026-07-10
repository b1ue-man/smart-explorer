pub(in crate::app) const DELETE_ERROR_LIMIT: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum DeleteKind {
    Recycle,
    Permanent,
}

impl DeleteKind {
    pub(in crate::app) fn success_text(self, count: usize) -> String {
        match self {
            Self::Recycle => format!("✓ {count} Eintrag/Einträge in den Papierkorb verschoben"),
            Self::Permanent => format!("✓ {count} Eintrag/Einträge endgültig gelöscht"),
        }
    }

    pub(in crate::app) fn error_context(self) -> &'static str {
        match self {
            Self::Recycle => "Papierkorb",
            Self::Permanent => "Endgültig löschen",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum DeleteOrigin {
    Explorer,
    Reclaim,
    Recovery,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum DeletePhase {
    Planning,
    Applying,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct DeleteProgress {
    pub(in crate::app) kind: DeleteKind,
    pub(in crate::app) origin: DeleteOrigin,
    pub(in crate::app) phase: DeletePhase,
    pub(in crate::app) targets_total: usize,
    pub(in crate::app) targets_processed: usize,
    pub(in crate::app) targets_succeeded: usize,
    pub(in crate::app) entries_planned: u64,
    pub(in crate::app) entries_deleted: u64,
    pub(in crate::app) current_path: String,
}

impl DeleteProgress {
    pub(in crate::app) fn new(
        kind: DeleteKind,
        origin: DeleteOrigin,
        targets_total: usize,
    ) -> Self {
        Self {
            kind,
            origin,
            phase: DeletePhase::Planning,
            targets_total,
            targets_processed: 0,
            targets_succeeded: 0,
            entries_planned: 0,
            entries_deleted: 0,
            current_path: String::new(),
        }
    }
}

pub(in crate::app) struct DeleteOutcome {
    pub(in crate::app) kind: DeleteKind,
    pub(in crate::app) origin: DeleteOrigin,
    pub(in crate::app) attempted: usize,
    pub(in crate::app) processed: usize,
    pub(in crate::app) succeeded: usize,
    pub(in crate::app) succeeded_paths: Vec<String>,
    pub(in crate::app) errors: Vec<(String, String)>,
    pub(in crate::app) suppressed_errors: u64,
    pub(in crate::app) canceled: bool,
    pub(in crate::app) entries_planned: u64,
    pub(in crate::app) entries_deleted: u64,
    pub(in crate::app) partial_mutation: bool,
}

impl DeleteOutcome {
    pub(in crate::app) fn new(kind: DeleteKind, origin: DeleteOrigin, attempted: usize) -> Self {
        Self {
            kind,
            origin,
            attempted,
            processed: 0,
            succeeded: 0,
            succeeded_paths: Vec::new(),
            errors: Vec::new(),
            suppressed_errors: 0,
            canceled: false,
            entries_planned: 0,
            entries_deleted: 0,
            partial_mutation: false,
        }
    }

    pub(in crate::app) fn record_success(&mut self, path: String) {
        self.processed = self.processed.saturating_add(1);
        self.succeeded = self.succeeded.saturating_add(1);
        self.succeeded_paths.push(path);
    }

    pub(in crate::app) fn record_error(&mut self, path: String, detail: String) {
        self.processed = self.processed.saturating_add(1);
        self.record_aux_error(path, detail);
    }

    pub(in crate::app) fn record_aux_error(&mut self, path: String, detail: String) {
        if self.errors.len() < DELETE_ERROR_LIMIT {
            self.errors.push((path, detail));
        } else {
            self.suppressed_errors = self.suppressed_errors.saturating_add(1);
        }
    }
}

pub(in crate::app) enum DeleteMsg {
    Progress(DeleteProgress),
    Finished(DeleteOutcome),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_confirmed_successes_enter_the_success_path_list() {
        let mut outcome = DeleteOutcome::new(DeleteKind::Permanent, DeleteOrigin::Explorer, 2);
        outcome.record_success("/done".to_string());
        outcome.record_error("/failed".to_string(), "injected".to_string());
        assert_eq!(outcome.processed, 2);
        assert_eq!(outcome.succeeded, 1);
        assert_eq!(outcome.succeeded_paths, vec!["/done"]);
    }

    #[test]
    fn auxiliary_errors_are_bounded_without_inflating_processed_targets() {
        let mut outcome = DeleteOutcome::new(DeleteKind::Recycle, DeleteOrigin::Reclaim, 0);
        for index in 0..(DELETE_ERROR_LIMIT + 5) {
            outcome.record_aux_error("journal".to_string(), index.to_string());
        }
        assert_eq!(outcome.processed, 0);
        assert_eq!(outcome.errors.len(), DELETE_ERROR_LIMIT);
        assert_eq!(outcome.suppressed_errors, 5);
    }
}
