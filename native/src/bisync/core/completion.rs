//! What apply reports while a run is still going (V3, FS1): every finished
//! action with the signatures the action itself observed, entries it had to
//! leave out or defer, and an early stop. The orchestration records finished
//! actions in baseline checkpoints, so a failure, cancel or crash never loses
//! what was already done.
use super::omissions::OmissionKind;
use super::run_types::RunStop;
use super::types::{PairSide, Sig};

/// What a finished action did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletedKind {
    /// A file was copied from side `from` to the other side (copy, update,
    /// winner of a keep-both conflict).
    Copied { from: PairSide },
    /// A file was copied and then removed from side `from` (move job), or a
    /// pending move finished its source deletion.
    Moved { from: PairSide },
    /// A file was removed from `side` (propagated deletion, mirror orphan).
    Deleted { side: PairSide },
    /// A folder was created on `side`.
    DirCreated { side: PairSide },
    /// An empty folder was removed from `side`.
    DirRemoved { side: PairSide },
}

/// One finished action, reported at once from apply's worker threads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletedAction {
    /// The action's rel (the planning identity, as in `Action`).
    pub rel: String,
    pub kind: CompletedKind,
    /// The source side after the action: the signature that was actually
    /// copied (verified against the plan by capture and revalidation); `None`
    /// when the source no longer exists (move, deletion, folder actions).
    pub src_sig: Option<Sig>,
    /// The destination side after the action (stat of the published file,
    /// with its transferred time); `None` after a deletion and for folders.
    /// Hashes follow the side's hash mode: non-zero when the run hashes that
    /// side (computed from the streamed bytes).
    pub dst_sig: Option<Sig>,
    /// The effect survives a power loss (data and directory entry flushed, or
    /// committed by the remote). `false` only for local Linux/Android writes
    /// that wait for the side's `syncfs`, which the checkpoint performs
    /// before it records them.
    pub durable: bool,
}

impl CompletedAction {
    /// The baseline entry (side A, side B) a finished file action leaves;
    /// `(None, None)` removes the entry. `None` for folder actions.
    pub fn baseline_entry(&self) -> Option<(Option<Sig>, Option<Sig>)> {
        match self.kind {
            CompletedKind::Copied { from: PairSide::A } => Some((self.src_sig, self.dst_sig)),
            CompletedKind::Copied { from: PairSide::B } => Some((self.dst_sig, self.src_sig)),
            CompletedKind::Moved { from: PairSide::A } => Some((None, self.dst_sig)),
            CompletedKind::Moved { from: PairSide::B } => Some((self.dst_sig, None)),
            CompletedKind::Deleted { .. } => Some((None, None)),
            CompletedKind::DirCreated { .. } | CompletedKind::DirRemoved { .. } => None,
        }
    }
}

/// A planned folder change (FS5): created before the copies into it,
/// removed after the deletions below it and only while it is empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirAction {
    Create { side: PairSide, rel: String },
    Remove { side: PairSide, rel: String },
}

impl DirAction {
    pub fn side(&self) -> PairSide {
        match self {
            DirAction::Create { side, .. } | DirAction::Remove { side, .. } => *side,
        }
    }

    pub fn rel(&self) -> &str {
        match self {
            DirAction::Create { rel, .. } | DirAction::Remove { rel, .. } => rel.as_str(),
        }
    }
}

/// Receives apply's results while the run goes on. Called from apply's
/// worker threads, so implementations synchronize themselves.
pub trait ApplySink: Sync {
    /// The action finished; its result enters the baseline once durable.
    fn completed(&self, action: CompletedAction);

    /// Nothing was done for `rel` because its entry cannot exist on the
    /// destination (too large, name impossible) or vanished before it was
    /// read; it is protected like a walk omission, not an error.
    fn omitted(&self, rel: &str, kind: OmissionKind) {
        let _ = (rel, kind);
    }

    /// `rel` changed after it was planned; nothing was committed and the next
    /// run handles it (not an error).
    fn deferred(&self, rel: &str, reason: &str) {
        let _ = (rel, reason);
    }

    /// Apply ended early; what it completed before stays recorded.
    fn stopped(&self, stop: RunStop) {
        let _ = stop;
    }
}
