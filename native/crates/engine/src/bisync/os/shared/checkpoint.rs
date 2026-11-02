//! Recording finished work while a run goes on (V3, FS1/B18): apply reports
//! every finished action at once through an [`ApplySink`]; the orchestration
//! turns durable results into baseline checkpoints (every N actions or T
//! seconds and once more at the end), so a failed, canceled or killed run
//! keeps everything it already did.
use std::sync::{Mutex, MutexGuard};

use super::completion::{ApplySink, CompletedAction};
use super::keys::Spellings;
use super::omissions::OmissionKind;
use super::run_types::RunStop;
use super::versions::RunVersions;

/// What apply receives from the orchestration besides the plan (V3).
pub(super) struct ApplyScope<'a> {
    /// Where finished actions, apply-time omissions, deferrals and an early
    /// stop go, the moment they happen.
    pub(super) sink: &'a dyn ApplySink,
    /// Versions of everything this run replaces or deletes.
    pub(super) versions: &'a RunVersions,
    /// Each side's own spelling of the planned rels.
    pub(super) spellings: &'a Spellings,
}

/// Everything an [`ApplySink`] was told.
#[derive(Clone, Debug, Default)]
pub(super) struct Collected {
    pub(super) completed: Vec<CompletedAction>,
    pub(super) omitted: Vec<(String, OmissionKind)>,
    pub(super) deferred: Vec<(String, String)>,
    pub(super) stop: Option<RunStop>,
}

/// A sink that only collects (previews, single actions, tests).
#[derive(Debug, Default)]
pub(super) struct CollectingSink {
    collected: Mutex<Collected>,
}

impl CollectingSink {
    fn lock(&self) -> MutexGuard<'_, Collected> {
        self.collected
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Everything reported so far; the sink starts empty again.
    pub(super) fn take(&self) -> Collected {
        std::mem::take(&mut *self.lock())
    }
}

impl ApplySink for CollectingSink {
    fn completed(&self, action: CompletedAction) {
        self.lock().completed.push(action);
    }

    fn omitted(&self, rel: &str, kind: OmissionKind) {
        self.lock().omitted.push((rel.to_string(), kind));
    }

    fn deferred(&self, rel: &str, reason: &str) {
        self.lock()
            .deferred
            .push((rel.to_string(), reason.to_string()));
    }

    fn stopped(&self, stop: RunStop) {
        let mut collected = self.lock();
        if collected.stop.is_none() {
            collected.stop = Some(stop);
        }
    }
}
