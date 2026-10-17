//! Plan rules of a job kind (vault plan C5.4 R4, OE4): a rule stage right
//! after planning and the order in which apply runs the planned actions.
//!
//! Without installed rules every job plans and applies exactly as before:
//! there is no rule stage, and apply runs one phase with all actions in plan
//! order through the unchanged pool call. A job kind installs its rules with
//! `enter` on the thread that runs, previews or applies; the engine asks
//! `current()` at its two hooks: the rule stage in
//! `orchestration_plan::prepare_spelled` (full scan and preview) and the
//! phases in `apply_reporting::run`.
use std::cell::RefCell;
use std::sync::Arc;

pub(crate) use super::plan_types::PairPlan;
pub(crate) use super::types::Action;

/// Reason reported for an action that the phases leave out of this run.
pub(crate) const DEFERRED_BY_RULE: &str = "Für den nächsten Lauf zurückgestellt (Regel der Jobart)";
/// Reason reported for an action held back after an earlier phase failed.
pub(crate) const HELD_BACK: &str =
    "Wartet auf eine nicht abgeschlossene Aktion einer früheren Phase; folgt im nächsten Lauf";

/// The rules of one job kind. The defaults have no effect.
pub(crate) trait PlanRules: Send + Sync {
    /// Rule stage on the finished plan of a full scan or a preview.
    fn adjust(&self, _plan: &mut PairPlan) {}

    /// Apply phases as index lists into `actions` (the actions this run
    /// applies, in plan order). A phase starts only after every action of
    /// the previous phase completed or failed. An action listed in no phase
    /// is not started and is reported as deferred (`deferral_reason`); a
    /// repeated or out-of-range index is ignored.
    fn phases(&self, actions: &[Action]) -> Vec<Vec<usize>> {
        vec![(0..actions.len()).collect()]
    }

    /// Of the actions of the coming phase, the positions in `coming` that
    /// wait for the next run because actions of earlier phases did not
    /// complete in this run (`incomplete`: failed, deferred, held back).
    fn held_back(&self, _incomplete: &[&Action], _coming: &[&Action]) -> Vec<usize> {
        Vec::new()
    }

    /// The reason reported for an action that no phase lists.
    fn deferral_reason(&self, _action: &Action) -> String {
        DEFERRED_BY_RULE.to_string()
    }
}

/// The rules of every existing job kind: no rule stage, one phase.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DefaultRules;

impl PlanRules for DefaultRules {}

/// Normalized apply phases: each action index at most once, empty phases
/// dropped; `deferred` lists the actions that no phase names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PhasePlan {
    pub(crate) phases: Vec<Vec<usize>>,
    pub(crate) deferred: Vec<usize>,
}

/// The phases `rules` give for `actions`, normalized; `None` when they are
/// the single phase of all actions in plan order, which the engine applies
/// exactly as without rules.
pub(crate) fn phase_plan(rules: &dyn PlanRules, actions: &[Action]) -> Option<PhasePlan> {
    let mut seen = vec![false; actions.len()];
    let mut phases = Vec::new();
    for phase in rules.phases(actions) {
        let mut kept = Vec::with_capacity(phase.len());
        for index in phase {
            if seen.get(index) == Some(&false) {
                seen[index] = true;
                kept.push(index);
            }
        }
        if !kept.is_empty() {
            phases.push(kept);
        }
    }
    let deferred: Vec<usize> = (0..actions.len()).filter(|index| !seen[*index]).collect();
    let unchanged = deferred.is_empty()
        && phases.len() <= 1
        && phases
            .first()
            .is_none_or(|phase| phase.iter().copied().eq(0..actions.len()));
    (!unchanged).then_some(PhasePlan { phases, deferred })
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<dyn PlanRules>>> = const { RefCell::new(None) };
}

/// Keeps rules installed on this thread; the previous rules return when it
/// drops.
pub(crate) struct CurrentRules {
    previous: Option<Arc<dyn PlanRules>>,
}

/// Installs `rules` for the runs, previews and applies on this thread.
#[must_use = "the rules apply only while the guard lives"]
pub(crate) fn enter(rules: Arc<dyn PlanRules>) -> CurrentRules {
    let previous = CURRENT.with(|current| current.replace(Some(rules)));
    CurrentRules { previous }
}

impl Drop for CurrentRules {
    fn drop(&mut self) {
        let previous = self.previous.take();
        CURRENT.with(|current| *current.borrow_mut() = previous);
    }
}

/// The rules installed on this thread; `None` = the defaults of every job
/// kind (no rule stage, one phase).
pub(crate) fn current() -> Option<Arc<dyn PlanRules>> {
    CURRENT.with(|current| current.borrow().clone())
}
