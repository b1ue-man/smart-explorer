//! Empty-folder passes by depth level (G4 MS10): creations run from the
//! shallowest level down (every parent before its children), removals from
//! the deepest level up (every child before its parent); the folders of one
//! level are independent and run on several threads. A level starts only
//! after the previous one finished, so the order guarantee of the former
//! serial pass holds.
use super::completion::DirAction;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Runs `each` for every action of `actions` level by level on up to
/// `workers` threads; `halted` ends the pass between two actions.
pub(super) fn run(
    actions: Vec<&DirAction>,
    creating: bool,
    workers: usize,
    halted: &(dyn Fn() -> bool + Sync),
    each: &(dyn Fn(&DirAction) + Sync),
) {
    let mut levels: BTreeMap<usize, Vec<&DirAction>> = BTreeMap::new();
    for action in actions {
        levels
            .entry(action.rel().matches('/').count())
            .or_default()
            .push(action);
    }
    let ordered: Vec<Vec<&DirAction>> = if creating {
        levels.into_values().collect()
    } else {
        levels.into_values().rev().collect()
    };
    for level in ordered {
        if halted() {
            return;
        }
        let next = AtomicUsize::new(0);
        let threads = workers.clamp(1, level.len());
        std::thread::scope(|scope| {
            for _ in 1..threads {
                scope.spawn(|| drain(&level, &next, halted, each));
            }
            drain(&level, &next, halted, each);
        });
    }
}

fn drain(
    level: &[&DirAction],
    next: &AtomicUsize,
    halted: &(dyn Fn() -> bool + Sync),
    each: &(dyn Fn(&DirAction) + Sync),
) {
    loop {
        if halted() {
            return;
        }
        let Some(action) = level.get(next.fetch_add(1, Ordering::Relaxed)) else {
            return;
        };
        each(action);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn speed_task_sync_folder_levels_keep_parents_before_children() {
        let actions = [
            DirAction::Create {
                rel: "a/b/c".into(),
                side: crate::bisync::PairSide::B,
            },
            DirAction::Create {
                rel: "a".into(),
                side: crate::bisync::PairSide::B,
            },
            DirAction::Create {
                rel: "a/b".into(),
                side: crate::bisync::PairSide::B,
            },
            DirAction::Create {
                rel: "z".into(),
                side: crate::bisync::PairSide::B,
            },
        ];
        for creating in [true, false] {
            let seen = Mutex::new(Vec::new());
            run(
                actions.iter().collect(),
                creating,
                4,
                &|| false,
                &|action| seen.lock().unwrap().push(action.rel().to_string()),
            );
            let seen = seen.into_inner().unwrap();
            let at = |rel: &str| seen.iter().position(|seen| seen == rel).unwrap();
            assert_eq!(seen.len(), 4);
            if creating {
                assert!(at("a") < at("a/b") && at("a/b") < at("a/b/c"));
            } else {
                assert!(at("a/b/c") < at("a/b") && at("a/b") < at("a"));
            }
        }
    }
}
