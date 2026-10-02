//! Sync runs share the transfer flows (`crate::transfer::flow`): one adaptive
//! limiter per connection or local volume decides how many operations run at
//! once, for GUI transfers, Explorer hand-offs, one-way mirrors and two-way
//! syncs alike, so a sync and a paste on one connection share it fairly.
//!
//! Which flows regulate a pair follows the transfer rule (plan K2): moving a
//! file between a local folder and a remote takes a permit of the remote's
//! flow only, so a busy disk never throttles an upload; two local folders take
//! their volumes' flows, two remotes both connections' flows. Work on one side
//! alone (a guarded delete) takes that side's own flow.
//!
//! Flows are process-local: sync jobs that run in the background service share
//! their flows with each other but not with the GUI process, whose flows learn
//! their own limits for the same connections (K24). Overload replies of the
//! server halve the limit in each process on its own.
use crate::transfer::{acquire_pair, flow_for, Flow, FlowPermit, PermitPair};
use crate::vfs::Backend;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Sync runs number their jobs from here, far above the ids the transfer
/// engine counts up from 1, so a sync never shares a turn with a transfer in
/// a flow's round robin (2^48 transfers are never reached).
const FIRST_SYNC_JOB: u64 = 1 << 48;
static NEXT_SYNC_JOB: AtomicU64 = AtomicU64::new(FIRST_SYNC_JOB);

/// A fresh job id for one sync run (fair turns between runs on one flow).
pub(crate) fn next_job() -> u64 {
    NEXT_SYNC_JOB.fetch_add(1, Ordering::Relaxed)
}

// One side of a sync pair: `A` is the source of a one-way mirror (the
// engine-wide type, re-exported for the flow users).
pub(crate) use super::types::PairSide;

/// The flows of both sides of one sync run.
pub(crate) struct PairFlows {
    a: Arc<Flow>,
    b: Arc<Flow>,
    a_regulates: bool,
    b_regulates: bool,
    job: u64,
}

impl PairFlows {
    pub(crate) fn new(a: &dyn Backend, root_a: &str, b: &dyn Backend, root_b: &str) -> Self {
        let (a_local, b_local) = (a.is_local(), b.is_local());
        Self {
            a: flow_for(a, root_a),
            b: flow_for(b, root_b),
            // A local side regulates only next to another local side.
            a_regulates: !a_local || b_local,
            b_regulates: !b_local || a_local,
            job: next_job(),
        }
    }

    pub(crate) fn flow(&self, side: PairSide) -> &Arc<Flow> {
        match side {
            PairSide::A => &self.a,
            PairSide::B => &self.b,
        }
    }

    fn regulates(&self, side: PairSide) -> bool {
        match side {
            PairSide::A => self.a_regulates,
            PairSide::B => self.b_regulates,
        }
    }

    /// Both sides are served by one connection (or one local volume).
    pub(crate) fn same_connection(&self) -> bool {
        self.a.key() == self.b.key()
    }

    /// A copy within ONE remote connection holds a reader and a writer of
    /// it at once; with pooled connections several such copies could each
    /// hold one and wait for another. There the protocol's own bound
    /// (`parallelism()`, the fixed limit syncs used before) caps the copies;
    /// every other pair follows its flows alone (`None`).
    pub(crate) fn shared_connection_cap(&self, a: &dyn Backend, b: &dyn Backend) -> Option<usize> {
        (self.same_connection() && !(a.is_local() && b.is_local()))
            .then(|| a.parallelism().min(b.parallelism()).max(1))
    }

    /// Permits for moving one file between the sides; `None` once canceled.
    /// A second flow is only taken when free (`acquire_pair`), so opposite
    /// syncs on the same two connections cannot deadlock.
    pub(crate) fn transfer(&self, cancel: &AtomicBool) -> Option<PermitPair> {
        let (one, other) = match (self.a_regulates, self.b_regulates) {
            (true, true) => (&self.a, Some(&self.b)),
            (true, false) => (&self.a, None),
            (false, _) => (&self.b, None),
        };
        acquire_pair(one, other, self.job, cancel)
    }

    /// A permit for work on `side` alone (a guarded delete with its backup
    /// read): that side's own flow, local or not.
    pub(crate) fn single(&self, side: PairSide, cancel: &AtomicBool) -> Option<PermitPair> {
        acquire_pair(self.flow(side), None, self.job, cancel)
    }

    /// A listing permit on `side`: `Some(None)` when that side does not
    /// regulate (a local folder next to a remote), `None` once canceled.
    /// Listings may use the flow's reserved metadata slot.
    pub(crate) fn listing(
        &self,
        side: PairSide,
        cancel: &AtomicBool,
    ) -> Option<Option<FlowPermit>> {
        if !self.regulates(side) {
            return Some(None);
        }
        self.flow(side).acquire_meta(cancel).map(Some)
    }

    /// The higher current limit of the regulating flows (at least one).
    pub(crate) fn limit(&self) -> usize {
        let limit_of = |side: PairSide| {
            if self.regulates(side) {
                self.flow(side).snapshot().limit
            } else {
                0
            }
        };
        limit_of(PairSide::A).max(limit_of(PairSide::B)).max(1)
    }
}
