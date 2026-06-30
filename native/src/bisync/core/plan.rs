use std::collections::BTreeSet;

use super::types::{
    Action, Baseline, BisyncOptions, CompareMode, Conflict, ConflictMode, DeletePolicy, Direction,
    Sig, Tree,
};

fn sig_mtime(s: Option<Sig>) -> i64 {
    s.map(|s| s.mtime_ms).unwrap_or(i64::MIN)
}
fn sig_size(s: Option<Sig>) -> u64 {
    s.map(|s| s.size).unwrap_or(0)
}

/// Comparison-mode-aware equality of two optional signatures.
pub(super) fn sig_eq(x: Option<Sig>, y: Option<Sig>, opts: &BisyncOptions) -> bool {
    match (x, y) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            if a.size != b.size {
                return false;
            }
            // Content-hash short-circuit: when BOTH sides carry a real content
            // hash (a server's free native MD5 and/or a cheap local read), equal
            // size+hash means identical content — independent of mtime. This is
            // what lets a local→Drive sync skip files whose mtime differs but
            // content matches (no re-transfer), in EVERY compare mode. Which
            // sides get a hash is decided per-mode by `hash_mode` at walk time, so
            // SizeOnly (no hashing) stays pure size and only the modes that should
            // use content do. A hash of 0 means "not hashed" → not eligible.
            if a.hash != 0 && b.hash != 0 {
                return a.hash == b.hash;
            }
            match opts.compare {
                CompareMode::SizeOnly => true,
                CompareMode::Checksum => a.hash == b.hash,
                CompareMode::MtimeSize => (a.mtime_ms - b.mtime_ms).abs() <= opts.modify_window_ms,
            }
        }
        _ => false,
    }
}

/// Decide the actions + conflicts from the two current trees and the baseline.
/// Returns (actions, conflicts, converged) where `converged` lists rels that are
/// now identical on both sides (baseline should record them).
pub fn plan(
    a: &Tree,
    b: &Tree,
    base: &Baseline,
    opts: BisyncOptions,
) -> (Vec<Action>, Vec<Conflict>, Vec<String>) {
    let mut actions = Vec::new();
    let mut conflicts = Vec::new();
    let mut converged = Vec::new();

    // Mirror: a stateless, one-way exact replica — the destination is made
    // identical to the source, deleting orphans. The baseline isn't consulted.
    if opts.delete == DeletePolicy::Mirror
        && matches!(opts.direction, Direction::AtoB | Direction::BtoA)
    {
        let atob = opts.direction == Direction::AtoB;
        let (src, dst) = if atob { (a, b) } else { (b, a) };
        let mut rels: BTreeSet<&String> = BTreeSet::new();
        rels.extend(src.keys());
        rels.extend(dst.keys());
        for rel in rels {
            let sn = src.get(rel).copied();
            let dn = dst.get(rel).copied();
            if sig_eq(sn, dn, &opts) {
                converged.push(rel.clone());
                continue;
            }
            match (sn, dn) {
                (Some(_), _) => actions.push(if atob {
                    Action::CopyAtoB(rel.clone())
                } else {
                    Action::CopyBtoA(rel.clone())
                }),
                (None, Some(_)) => actions.push(if atob {
                    Action::DeleteB(rel.clone())
                } else {
                    Action::DeleteA(rel.clone())
                }),
                (None, None) => {}
            }
        }
        return (actions, conflicts, converged);
    }

    let mut rels: BTreeSet<&String> = BTreeSet::new();
    rels.extend(a.keys());
    rels.extend(b.keys());
    rels.extend(base.keys());

    let allow_a_to_b = matches!(opts.direction, Direction::AtoB | Direction::Both);
    let allow_b_to_a = matches!(opts.direction, Direction::BtoA | Direction::Both);
    let allow_delete = opts.delete != DeletePolicy::NoDelete;

    for rel in rels {
        let an = a.get(rel).copied();
        let bn = b.get(rel).copied();
        let (ba, bb) = base.get(rel).copied().unwrap_or((None, None));
        let a_changed = !sig_eq(an, ba, &opts);
        let b_changed = !sig_eq(bn, bb, &opts);

        if !a_changed && !b_changed {
            continue; // in sync per the baseline
        }
        // Both sides ended up identical (e.g. same edit on both) → no work.
        if sig_eq(an, bn, &opts) {
            converged.push(rel.clone());
            continue;
        }

        match (a_changed, b_changed) {
            (true, false) => {
                // propagate A's state to B
                if allow_a_to_b {
                    match an {
                        Some(_) => actions.push(Action::CopyAtoB(rel.clone())),
                        None => {
                            if allow_delete {
                                actions.push(Action::DeleteB(rel.clone()))
                            }
                        }
                    }
                }
            }
            (false, true) => {
                if allow_b_to_a {
                    match bn {
                        Some(_) => actions.push(Action::CopyBtoA(rel.clone())),
                        None => {
                            if allow_delete {
                                actions.push(Action::DeleteA(rel.clone()))
                            }
                        }
                    }
                }
            }
            (true, true) => {
                if opts.conflict == ConflictMode::FileLevel {
                    conflicts.push(Conflict {
                        rel: rel.clone(),
                        a: an,
                        b: bn,
                    });
                } else if opts.conflict == ConflictMode::KeepBoth {
                    // Winner (newer) keeps the name; loser preserved as a copy.
                    let a_wins = match (an, bn) {
                        (Some(_), None) => true,
                        (None, Some(_)) => false,
                        (Some(sa), Some(sb)) => sa.mtime_ms >= sb.mtime_ms,
                        (None, None) => continue,
                    };
                    if a_wins && allow_a_to_b {
                        actions.push(Action::KeepBothAtoB(rel.clone()));
                    } else if !a_wins && allow_b_to_a {
                        actions.push(Action::KeepBothBtoA(rel.clone()));
                    }
                } else {
                    // A deterministic winner side from the policy.
                    let a_wins = match opts.conflict {
                        ConflictMode::SourceWins => true,
                        ConflictMode::DestWins => false,
                        ConflictMode::NewerWins => sig_mtime(an) >= sig_mtime(bn),
                        ConflictMode::OlderWins => sig_mtime(an) <= sig_mtime(bn),
                        ConflictMode::LargerWins => sig_size(an) >= sig_size(bn),
                        ConflictMode::SmallerWins => sig_size(an) <= sig_size(bn),
                        _ => true,
                    };
                    if a_wins {
                        if allow_a_to_b {
                            match an {
                                Some(_) => actions.push(Action::CopyAtoB(rel.clone())),
                                None => {
                                    if allow_delete {
                                        actions.push(Action::DeleteB(rel.clone()))
                                    }
                                }
                            }
                        }
                    } else if allow_b_to_a {
                        match bn {
                            Some(_) => actions.push(Action::CopyBtoA(rel.clone())),
                            None => {
                                if allow_delete {
                                    actions.push(Action::DeleteA(rel.clone()))
                                }
                            }
                        }
                    }
                }
            }
            (false, false) => unreachable!(),
        }
    }
    (actions, conflicts, converged)
}

/// Build the new baseline after a run: successful actions + converged rels are
/// now in sync (record both sides' current sig); conflicts, failed actions, and
/// skipped rels keep their previous baseline so they're re-detected next time.
pub fn update_baseline(
    base: &Baseline,
    a: &Tree,
    b: &Tree,
    applied: &[Action],
    converged: &[String],
    conflicts: &[Conflict],
) -> Baseline {
    let conflict_set: BTreeSet<&str> = conflicts.iter().map(|c| c.rel.as_str()).collect();
    let mut nb = base.clone();
    let mut record = |rel: &str| {
        nb.insert(rel.to_string(), (a.get(rel).copied(), b.get(rel).copied()));
    };
    for act in applied {
        let rel = match act {
            Action::CopyAtoB(r)
            | Action::CopyBtoA(r)
            | Action::DeleteA(r)
            | Action::DeleteB(r)
            | Action::KeepBothAtoB(r)
            | Action::KeepBothBtoA(r) => r,
        };
        // After a copy both sides match; after a delete both are absent. For
        // NewerWins the loser side may not match yet — record current state so
        // the next walk reconciles. (Deletes leave the entry absent.)
        record(rel);
    }
    for rel in converged {
        record(rel);
    }
    // Drop entries that are now absent on both sides and not in conflict.
    nb.retain(|rel, (x, y)| x.is_some() || y.is_some() || conflict_set.contains(rel.as_str()));
    nb
}
