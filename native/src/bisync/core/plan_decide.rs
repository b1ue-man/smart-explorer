//! What happens to one planning key (V3): the classic decisions plus one-way
//! repair of destination drift (Y40), modification before deletion under the
//! size and time policies (Y41), finishing a verified move (Y59), mirrors in
//! sync by their baseline (FS2) and reading equal-size files before they
//! become conflicts or copies (FS1).
use super::compare::{same_entry, same_file, worth_hashing, Against};
use super::plan_types::PlanContext;
use super::types::{ConflictMode, DeletePolicy, Direction, PairSide, Sig};

/// One change to make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Copy { from: PairSide },
    FinalizeMove { from: PairSide },
    Delete { side: PairSide },
    KeepBoth { winner: PairSide },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Decision {
    /// In sync per the baseline: nothing to do, the entry stays.
    Nothing,
    Act(Step),
    Conflict,
    /// Record the current state without an action.
    Record,
    /// Gone on both sides: drop the baseline entry.
    Forget,
    /// Read both sides and decide again.
    Verify,
}

/// What both sides and the baseline hold for one key.
#[derive(Clone, Copy, Debug)]
pub(super) struct Observed {
    pub(super) a: Option<Sig>,
    pub(super) b: Option<Sig>,
    pub(super) base: Option<(Option<Sig>, Option<Sig>)>,
}

impl Observed {
    fn sig(&self, side: PairSide) -> Option<Sig> {
        match side {
            PairSide::A => self.a,
            PairSide::B => self.b,
        }
    }

    fn base(&self, side: PairSide) -> Option<Sig> {
        self.base.and_then(|(a, b)| match side {
            PairSide::A => a,
            PairSide::B => b,
        })
    }

    /// Both sides still look as recorded at the last run.
    fn matches_baseline(&self, ctx: &PlanContext<'_>) -> bool {
        self.base.is_some()
            && [PairSide::A, PairSide::B].into_iter().all(|side| {
                same_entry(
                    self.sig(side),
                    self.base(side),
                    ctx.times.side(side),
                    &ctx.opts,
                    Against::Baseline,
                )
            })
    }
}

/// The side a one-way job copies from (`None` for two-way).
pub(super) fn source_side(direction: Direction) -> Option<PairSide> {
    match direction {
        Direction::AtoB => Some(PairSide::A),
        Direction::BtoA => Some(PairSide::B),
        Direction::Both => None,
    }
}

/// Whether changes may flow from `from` to the other side.
pub(super) fn may_send(direction: Direction, from: PairSide) -> bool {
    source_side(direction).is_none_or(|source| source == from)
}

pub(super) fn decide(seen: &Observed, ctx: &PlanContext<'_>) -> Decision {
    let opts = &ctx.opts;
    if let Some(source) = source_side(opts.direction) {
        if opts.move_files {
            if let Some(decision) = decide_move(seen, source, ctx) {
                return decision;
            }
        }
        if opts.delete == DeletePolicy::Mirror {
            return decide_mirror(seen, source, ctx);
        }
    }
    let changed = |side: PairSide| {
        !same_entry(
            seen.sig(side),
            seen.base(side),
            ctx.times.side(side),
            opts,
            Against::Baseline,
        )
    };
    let (a_changed, b_changed) = (changed(PairSide::A), changed(PairSide::B));
    if !a_changed && !b_changed {
        return Decision::Nothing;
    }
    if same_entry(seen.a, seen.b, ctx.times.cross(), opts, Against::OtherSide) {
        return if seen.a.is_none() {
            Decision::Forget
        } else {
            Decision::Record
        };
    }
    match (a_changed, b_changed) {
        (true, false) => propagate(seen, PairSide::A, ctx),
        (false, true) => propagate(seen, PairSide::B, ctx),
        _ => both_changed(seen, ctx),
    }
}

/// Only `changed` differs from the baseline.
fn propagate(seen: &Observed, changed: PairSide, ctx: &PlanContext<'_>) -> Decision {
    let other = changed.other();
    if may_send(ctx.opts.direction, changed) {
        return match seen.sig(changed) {
            Some(_) => Decision::Act(Step::Copy { from: changed }),
            None if ctx.opts.delete != DeletePolicy::NoDelete => {
                Decision::Act(Step::Delete { side: other })
            }
            None => Decision::Record,
        };
    }
    // The destination of a one-way job drifted (Y40): repair it from the
    // source; an extra file the source never had stays.
    match seen.sig(other) {
        Some(_) => Decision::Act(Step::Copy { from: other }),
        None => Decision::Record,
    }
}

fn both_changed(seen: &Observed, ctx: &PlanContext<'_>) -> Decision {
    if ctx.can_hash && worth_hashing(seen.a, seen.b, &ctx.opts) {
        return Decision::Verify;
    }
    match source_side(ctx.opts.direction) {
        Some(source) => one_way_both_changed(seen, source, ctx),
        None => two_way_both_changed(seen, ctx),
    }
}

/// One-way jobs keep the destination current (Y40): the source wins unless
/// a policy explicitly prefers the destination; a deletion never beats a
/// modification under the size and time policies (Y41).
fn one_way_both_changed(seen: &Observed, source: PairSide, ctx: &PlanContext<'_>) -> Decision {
    let destination = source.other();
    match ctx.opts.conflict {
        // Repair of destination-only drift is handled by `propagate` above.
        // Actual edits on both sides retain the established strict conflict.
        ConflictMode::FileLevel => Decision::Conflict,
        ConflictMode::KeepBoth => match seen.sig(source) {
            Some(_) => Decision::Act(Step::KeepBoth { winner: source }),
            None => Decision::Record,
        },
        mode => {
            let winner = winner_under(mode, seen);
            if winner == destination {
                Decision::Record
            } else {
                finish_for(seen, winner, ctx)
            }
        }
    }
}

fn two_way_both_changed(seen: &Observed, ctx: &PlanContext<'_>) -> Decision {
    match ctx.opts.conflict {
        ConflictMode::FileLevel => Decision::Conflict,
        ConflictMode::KeepBoth => {
            let winner = match (seen.a, seen.b) {
                (Some(_), None) => PairSide::A,
                (None, Some(_)) => PairSide::B,
                (Some(a), Some(b)) if a.mtime_ms >= b.mtime_ms => PairSide::A,
                (Some(_), Some(_)) => PairSide::B,
                (None, None) => return Decision::Forget,
            };
            Decision::Act(Step::KeepBoth { winner })
        }
        mode => finish_for(seen, winner_under(mode, seen), ctx),
    }
}

/// The side a policy picks. With one side deleted, the modified side wins
/// under the size and time policies (Y41); source and destination
/// preferences stay explicit.
fn winner_under(mode: ConflictMode, seen: &Observed) -> PairSide {
    let prefer_a = match (mode, seen.a, seen.b) {
        (ConflictMode::SourceWins, _, _) => true,
        (ConflictMode::DestWins, _, _) => false,
        (_, Some(_), None) => true,
        (_, None, Some(_)) => false,
        (ConflictMode::NewerWins, Some(a), Some(b)) => a.mtime_ms >= b.mtime_ms,
        (ConflictMode::OlderWins, Some(a), Some(b)) => a.mtime_ms <= b.mtime_ms,
        (ConflictMode::LargerWins, Some(a), Some(b)) => a.size >= b.size,
        (ConflictMode::SmallerWins, Some(a), Some(b)) => a.size <= b.size,
        _ => true,
    };
    if prefer_a {
        PairSide::A
    } else {
        PairSide::B
    }
}

/// Makes the other side look like `winner`.
fn finish_for(seen: &Observed, winner: PairSide, ctx: &PlanContext<'_>) -> Decision {
    if !may_send(ctx.opts.direction, winner) {
        return Decision::Record;
    }
    match seen.sig(winner) {
        Some(_) => Decision::Act(Step::Copy { from: winner }),
        None if ctx.opts.delete != DeletePolicy::NoDelete => Decision::Act(Step::Delete {
            side: winner.other(),
        }),
        None => Decision::Record,
    }
}

/// Move jobs: a destination-only file is the finished state; a source file
/// whose destination holds the same content (or both still look as recorded
/// after the copy) only needs its verified source deletion (Y59).
fn decide_move(seen: &Observed, source: PairSide, ctx: &PlanContext<'_>) -> Option<Decision> {
    let destination = source.other();
    let Some(src) = seen.sig(source) else {
        return Some(if seen.sig(destination).is_some() {
            Decision::Record
        } else {
            Decision::Forget
        });
    };
    let dst = seen.sig(destination)?;
    if same_file(src, dst, ctx.times.cross(), &ctx.opts, Against::OtherSide)
        || seen.matches_baseline(ctx)
    {
        return Some(Decision::Act(Step::FinalizeMove { from: source }));
    }
    (ctx.can_hash && worth_hashing(Some(src), Some(dst), &ctx.opts)).then_some(Decision::Verify)
}

/// Mirror jobs make the destination an exact copy; a pair that still looks as
/// recorded after the last copy is in sync even where the destination cannot
/// keep the source's times (FS2).
fn decide_mirror(seen: &Observed, source: PairSide, ctx: &PlanContext<'_>) -> Decision {
    let destination = source.other();
    match (seen.sig(source), seen.sig(destination)) {
        (None, None) => Decision::Forget,
        (Some(_), None) => Decision::Act(Step::Copy { from: source }),
        (None, Some(_)) => Decision::Act(Step::Delete { side: destination }),
        (Some(src), Some(dst)) => {
            if same_file(src, dst, ctx.times.cross(), &ctx.opts, Against::OtherSide) {
                return Decision::Record;
            }
            if seen.matches_baseline(ctx) {
                return Decision::Nothing;
            }
            if ctx.can_hash && worth_hashing(Some(src), Some(dst), &ctx.opts) {
                return Decision::Verify;
            }
            Decision::Act(Step::Copy { from: source })
        }
    }
}
