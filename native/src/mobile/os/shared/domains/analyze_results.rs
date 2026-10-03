//! Results of `analyze.*` and `reclaim.*` kept for drill-down. A running
//! scan holds its slot; a finished result stays until the app releases it
//! (`analyze.release`) or memory runs short, then the oldest finished results
//! go first and the newest one always stays. A scan that ends without a
//! result (error, cancel, panic) frees its slot at once.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use crate::analytics::{Approximations, DuplicateGroup, DuplicateSummary, ScanOutcome, SizeNode};
use crate::mobile::ApiError;

/// Heap cost of one size node besides its name: the node and the
/// allocator's share of the name allocation.
const NODE_BYTES: u64 = std::mem::size_of::<SizeNode>() as u64 + 16;
/// Heap cost of one duplicate item besides its texts.
const ITEM_BYTES: u64 = 160;

pub(super) enum Stored {
    Analysis {
        outcome: ScanOutcome,
        approx: Approximations,
        base: String,
        root: String,
    },
    Duplicates {
        groups: Vec<DuplicateGroup>,
        summary: DuplicateSummary,
        base: String,
        recycling: bool,
    },
    Recycled {
        moved: Vec<String>,
    },
}

struct Slot {
    token: u64,
    task: Option<String>,
    result: Option<Stored>,
    /// Estimated heap bytes of `result`.
    bytes: u64,
}

static RESULTS: Mutex<VecDeque<Slot>> = Mutex::new(VecDeque::new());
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

fn with_results<T>(work: impl FnOnce(&mut VecDeque<Slot>) -> T) -> T {
    work(&mut RESULTS.lock().unwrap_or_else(PoisonError::into_inner))
}

/// The slot of a starting scan; dropped without [`Pending::store`] it frees
/// the slot, so a failed, canceled or panicked scan leaves nothing behind.
pub(super) struct Pending {
    token: u64,
}

impl Pending {
    pub(super) fn open() -> Self {
        let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
        with_results(|slots| {
            slots.push_back(Slot {
                token,
                task: None,
                result: None,
                bytes: 0,
            })
        });
        Self { token }
    }

    pub(super) fn token(&self) -> u64 {
        self.token
    }

    /// Keeps `result` for drill-down. Finished results of earlier scans make
    /// room while the kept ones exceed the memory budget.
    pub(super) fn store(self, result: Stored) {
        let bytes = result_bytes(&result);
        let token = self.token;
        with_results(|slots| {
            if let Some(slot) = slots.iter_mut().find(|slot| slot.token == token) {
                slot.result = Some(result);
                slot.bytes = bytes;
            }
            trim(slots, token, crate::transfer::memory_budget());
        });
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        let token = self.token;
        with_results(|slots| slots.retain(|slot| slot.token != token || slot.result.is_some()));
    }
}

/// Names the task of a slot (the task id exists only after the spawn).
pub(super) fn bind_task(token: u64, task: &str) {
    with_results(|slots| {
        if let Some(slot) = slots.iter_mut().find(|slot| slot.token == token) {
            slot.task = Some(task.to_string());
        }
    });
}

pub(super) fn with_stored<T>(
    task: &str,
    work: impl FnOnce(&Stored) -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    with_results(|slots| {
        let slot = slots
            .iter()
            .find(|slot| slot.task.as_deref() == Some(task))
            .ok_or_else(|| ApiError::new("not_found", "Ergebnis nicht mehr vorhanden."))?;
        match &slot.result {
            Some(result) => work(result),
            None => Err(ApiError::new("busy", "Der Scan läuft noch.")),
        }
    })
}

pub(super) fn with_stored_mut<T>(
    task: &str,
    work: impl FnOnce(&mut Stored) -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    with_results(|slots| {
        let slot = slots
            .iter_mut()
            .find(|slot| slot.task.as_deref() == Some(task))
            .ok_or_else(|| ApiError::new("not_found", "Ergebnis nicht mehr vorhanden."))?;
        let stored = slot
            .result
            .as_mut()
            .ok_or_else(|| ApiError::new("busy", "Der Scan läuft noch."))?;
        let result = work(stored);
        slot.bytes = result_bytes(stored);
        result
    })
}

/// Drops the result of `task` (also a running scan's future result);
/// `false` when nothing was kept for it.
pub(super) fn release(task: &str) -> bool {
    with_results(|slots| {
        let before = slots.len();
        slots.retain(|slot| slot.task.as_deref() != Some(task));
        slots.len() != before
    })
}

/// Drops the oldest finished results (never `keep`) while all kept results
/// together need more than `budget` bytes.
fn trim(slots: &mut VecDeque<Slot>, keep: u64, budget: u64) {
    let mut total = slots
        .iter()
        .fold(0u64, |total, slot| total.saturating_add(slot.bytes));
    let mut index = 0;
    while total > budget && index < slots.len() {
        let recycling = matches!(
            &slots[index].result,
            Some(Stored::Duplicates {
                recycling: true,
                ..
            })
        );
        if slots[index].result.is_some() && slots[index].token != keep && !recycling {
            total = total.saturating_sub(slots[index].bytes);
            slots.remove(index);
        } else {
            index += 1;
        }
    }
}

fn result_bytes(result: &Stored) -> u64 {
    let heap = match result {
        Stored::Analysis {
            outcome,
            approx,
            base,
            root,
        } => outcome
            .tree
            .as_ref()
            .map_or(0, tree_bytes)
            .saturating_add(approx.estimated_heap_bytes())
            .saturating_add(
                outcome
                    .platform
                    .as_ref()
                    .map_or(0, |figures| figures.estimated_heap_bytes()),
            )
            .saturating_add(strings_bytes(&outcome.notes))
            .saturating_add(vec_bytes(&outcome.issues))
            .saturating_add(outcome.issues.iter().fold(0u64, |total, issue| {
                total
                    .saturating_add(issue.path.capacity() as u64)
                    .saturating_add(issue.detail.capacity() as u64)
            }))
            .saturating_add(vec_bytes(&outcome.protected))
            .saturating_add(base.capacity() as u64)
            .saturating_add(root.capacity() as u64),
        Stored::Recycled { moved } => moved.iter().fold(0, |total: u64, path| {
            total.saturating_add(32 + path.len() as u64)
        }),
        Stored::Duplicates {
            groups,
            summary,
            base,
            ..
        } => groups
            .iter()
            .fold(0u64, |total, group| {
                group.items.iter().fold(
                    total.saturating_add(ITEM_BYTES + group.hash.hex.len() as u64),
                    |total, item| {
                        let texts = item.path.len()
                            + item.name.len()
                            + item.reason.len()
                            + item.backend_id.as_ref().map_or(0, String::len);
                        total.saturating_add(ITEM_BYTES + texts as u64)
                    },
                )
            })
            .saturating_add(strings_bytes(&summary.errors))
            .saturating_add(strings_bytes(&summary.limits))
            .saturating_add(vec_bytes(&summary.protected))
            .saturating_add(base.capacity() as u64),
    };
    (std::mem::size_of::<Stored>() as u64).saturating_add(heap)
}

fn vec_bytes<T>(items: &Vec<T>) -> u64 {
    (items.capacity() as u64).saturating_mul(std::mem::size_of::<T>() as u64)
}

fn strings_bytes(items: &Vec<String>) -> u64 {
    items.iter().fold(vec_bytes(items), |total, text| {
        total.saturating_add(text.capacity() as u64)
    })
}

/// Heap bytes of a size tree; the walk keeps one iterator per level only.
fn tree_bytes(tree: &SizeNode) -> u64 {
    let node = |node: &SizeNode| {
        NODE_BYTES
            .saturating_add(node.name.capacity() as u64)
            .saturating_add(
                (node.children.capacity().saturating_sub(node.children.len()) as u64)
                    .saturating_mul(std::mem::size_of::<SizeNode>() as u64),
            )
    };
    let mut total = node(tree);
    let mut levels = vec![tree.children.iter()];
    while let Some(children) = levels.last_mut() {
        match children.next() {
            Some(child) => {
                total = total.saturating_add(node(child));
                levels.push(child.children.iter());
            }
            None => {
                levels.pop();
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> SizeNode {
        SizeNode {
            name: name.into(),
            size: 1,
            is_dir: false,
            children: Vec::new(),
        }
    }

    fn slot(token: u64, bytes: u64, finished: bool) -> Slot {
        Slot {
            token,
            task: Some(format!("t{token}")),
            result: finished.then(|| Stored::Analysis {
                outcome: ScanOutcome::complete(file("x")),
                approx: Approximations::default(),
                base: String::new(),
                root: String::new(),
            }),
            bytes,
        }
    }

    #[test]
    fn review_task_results_trim_oldest_finished_within_budget() {
        let mut slots: VecDeque<Slot> = [
            slot(1, 60, true),
            slot(2, 10, false),
            slot(3, 50, true),
            slot(4, 40, true),
        ]
        .into();
        trim(&mut slots, 4, 100);
        let kept: Vec<u64> = slots.iter().map(|slot| slot.token).collect();
        // 1 goes (oldest finished); the running 2 and the newest 4 stay.
        assert_eq!(kept, [2, 3, 4]);
        trim(&mut slots, 4, 0);
        let kept: Vec<u64> = slots.iter().map(|slot| slot.token).collect();
        assert_eq!(kept, [2, 4], "the kept result and running scans survive");
    }

    #[test]
    fn review_task_results_tree_bytes_count_every_node() {
        let tree = SizeNode {
            name: "root".into(),
            size: 3,
            is_dir: true,
            children: vec![
                SizeNode {
                    name: "dir".into(),
                    size: 2,
                    is_dir: true,
                    children: vec![file("a"), file("bb")],
                },
                file("c"),
            ],
        };
        let names = ("root".len() + "dir".len() + "a".len() + "bb".len() + "c".len()) as u64;
        assert_eq!(tree_bytes(&tree), 5 * NODE_BYTES + names);
    }

    #[test]
    fn review_task_host_app_figures_participate_in_result_retention() {
        use crate::analytics::{PlatformFigures, PlatformTotals, VolumeRoot};
        let place = VolumeRoot {
            whole_volume: true,
            app_data: Some(Vec::new()),
        };
        let mut totals = PlatformTotals::default();
        totals.add_app(
            "host.package".repeat(1024),
            "Host app".repeat(1024),
            50,
            25,
            5,
        );
        let mut outcome = ScanOutcome::complete(file("root"));
        outcome.platform = Some(PlatformFigures::new(&place, &totals, 0));
        let approx = Approximations::compute(outcome.tree.as_ref().unwrap(), &place, totals, true);
        let expected = approx.estimated_heap_bytes()
            + outcome.platform.as_ref().unwrap().estimated_heap_bytes();
        let result = Stored::Analysis {
            outcome,
            approx,
            base: "share://direct/host/root".into(),
            root: "/root".into(),
        };
        let bytes = result_bytes(&result);
        assert!(bytes >= expected && expected > 20_000);
        let mut old = slot(1, bytes, true);
        old.result = Some(result);
        let mut slots: VecDeque<Slot> = [old, slot(2, 100, true)].into();
        trim(&mut slots, 2, bytes);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].token, 2);
    }

    #[test]
    fn review_task_results_pending_slot_frees_without_result() {
        let pending = Pending::open();
        let token = pending.token();
        bind_task(token, "review-task-pending");
        assert!(with_stored("review-task-pending", |_| Ok(())).is_err());
        drop(pending);
        let error = with_stored("review-task-pending", |_| Ok(())).unwrap_err();
        assert_eq!(error.kind, "not_found");

        let pending = Pending::open();
        bind_task(pending.token(), "review-task-stored");
        pending.store(Stored::Analysis {
            outcome: ScanOutcome::complete(file("y")),
            approx: Approximations::default(),
            base: String::new(),
            root: String::new(),
        });
        assert!(with_stored("review-task-stored", |_| Ok(())).is_ok());
        assert!(release("review-task-stored"));
        assert!(!release("review-task-stored"));
        assert!(with_stored("review-task-stored", |_| Ok(())).is_err());
    }
}
