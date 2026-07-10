use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

use super::{Backend, VfsResult};

const MAX_DELETE_ENTRIES: u64 = 1_000_000;
const MAX_DELETE_TEXT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_DELETE_DEPTH: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteTarget {
    pub path: String,
    pub id: Option<String>,
    pub is_dir: bool,
    pub is_symlink: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecursiveDeletePhase {
    Planning,
    Applying,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecursiveDeleteProgress {
    pub phase: RecursiveDeletePhase,
    pub planned: u64,
    pub removed: u64,
    pub current: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecursiveDeleteStatus {
    Complete,
    Canceled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecursiveDeleteReport {
    pub status: RecursiveDeleteStatus,
    pub planned: u64,
    pub removed: u64,
}

#[derive(Debug)]
pub struct RecursiveDeleteFailure {
    pub error: io::Error,
    pub planned: u64,
    pub removed: u64,
}

impl std::fmt::Display for RecursiveDeleteFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.error)
    }
}

impl std::error::Error for RecursiveDeleteFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

pub fn remove_entry(backend: &dyn Backend, target: &DeleteTarget) -> VfsResult<()> {
    let cancel = AtomicBool::new(false);
    match remove_entry_controlled(backend, target, &cancel, |_| {}) {
        Ok(report) if report.status == RecursiveDeleteStatus::Complete => Ok(()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "recursive delete was canceled",
        )),
        Err(failure) => Err(failure.error),
    }
}

pub fn remove_entry_controlled<F>(
    backend: &dyn Backend,
    target: &DeleteTarget,
    cancel: &AtomicBool,
    mut progress: F,
) -> Result<RecursiveDeleteReport, RecursiveDeleteFailure>
where
    F: FnMut(RecursiveDeleteProgress),
{
    let mut budget = DeleteBudget::default();
    let plan = match collect_delete_plan(backend, target, cancel, &mut budget, &mut progress) {
        Ok(Some(plan)) => plan,
        Ok(None) => return Ok(canceled_report(&budget, 0)),
        Err(error) => return Err(delete_failure(error, &budget, 0)),
    };

    let mut removed = 0u64;
    for item in plan {
        if cancel.load(Ordering::Acquire) {
            return Ok(canceled_report(&budget, removed));
        }
        progress(RecursiveDeleteProgress {
            phase: RecursiveDeletePhase::Applying,
            planned: budget.entries,
            removed,
            current: item.path.clone(),
        });
        if cancel.load(Ordering::Acquire) {
            return Ok(canceled_report(&budget, removed));
        }
        if let Err(error) = apply_planned_item(backend, &item) {
            return Err(delete_failure(error, &budget, removed));
        }
        removed = removed.saturating_add(1);
        progress(RecursiveDeleteProgress {
            phase: RecursiveDeletePhase::Applying,
            planned: budget.entries,
            removed,
            current: item.path,
        });
    }
    Ok(RecursiveDeleteReport {
        status: RecursiveDeleteStatus::Complete,
        planned: budget.entries,
        removed,
    })
}

#[derive(Default)]
struct DeleteBudget {
    entries: u64,
    text_bytes: u64,
}

impl DeleteBudget {
    fn record(&mut self, path: &str, depth: usize) -> VfsResult<()> {
        if depth > MAX_DELETE_DEPTH {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("delete tree exceeds {MAX_DELETE_DEPTH} levels"),
            ));
        }
        self.entries = self.entries.checked_add(1).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "delete entry count overflow")
        })?;
        self.text_bytes = self
            .text_bytes
            .checked_add(path.len() as u64)
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "delete path budget overflow")
            })?;
        if self.entries > MAX_DELETE_ENTRIES || self.text_bytes > MAX_DELETE_TEXT_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "delete tree exceeds its bounded collection budget",
            ));
        }
        Ok(())
    }
}

enum Pending {
    Inspect { target: DeleteTarget, depth: usize },
    Finish(DeleteTarget),
}

fn collect_delete_plan<F>(
    backend: &dyn Backend,
    target: &DeleteTarget,
    cancel: &AtomicBool,
    budget: &mut DeleteBudget,
    progress: &mut F,
) -> VfsResult<Option<Vec<DeleteTarget>>>
where
    F: FnMut(RecursiveDeleteProgress),
{
    budget.record(&target.path, 0)?;
    report_planning(progress, budget, &target.path);
    let mut plan = Vec::new();
    let mut pending = vec![Pending::Inspect {
        target: target.clone(),
        depth: 0,
    }];
    while let Some(next) = pending.pop() {
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let (target, depth) = match next {
            Pending::Finish(target) => {
                plan.push(target);
                continue;
            }
            Pending::Inspect { target, depth } => (target, depth),
        };
        let current = backend.stat(&target.path)?;
        if current.is_symlink || !current.is_dir {
            plan.push(DeleteTarget {
                path: target.path,
                id: current.id.or(target.id),
                is_dir: false,
                is_symlink: current.is_symlink,
            });
            continue;
        }
        if !target.is_dir || target.is_symlink {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "entry changed into a directory before deletion: {}",
                    target.path
                ),
            ));
        }
        let children = backend.list_dir(&target.path)?;
        let child_depth = depth
            .checked_add(1)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "delete depth overflow"))?;
        let mut child_names = std::collections::HashSet::new();
        let mut child_targets = Vec::with_capacity(children.len());
        for child in children {
            if cancel.load(Ordering::Acquire) {
                return Ok(None);
            }
            validate_child_name(&child.name)?;
            if !child_names.insert(child.name.clone()) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "backend returned duplicate child name in {}: {:?}",
                        target.path, child.name
                    ),
                ));
            }
            let child_path = join_child(&target.path, &child.name);
            budget.record(&child_path, child_depth)?;
            report_planning(progress, budget, &child_path);
            child_targets.push(DeleteTarget {
                path: child_path,
                id: child.id,
                is_dir: child.is_dir,
                is_symlink: child.is_symlink,
            });
        }
        pending.push(Pending::Finish(DeleteTarget {
            path: target.path,
            id: current.id.or(target.id),
            is_dir: true,
            is_symlink: false,
        }));
        pending.extend(
            child_targets
                .into_iter()
                .rev()
                .map(|target| Pending::Inspect {
                    target,
                    depth: child_depth,
                }),
        );
    }
    Ok(Some(plan))
}

fn apply_planned_item(backend: &dyn Backend, item: &DeleteTarget) -> VfsResult<()> {
    let fresh = backend.stat(&item.path)?;
    if let (Some(expected), Some(actual)) = (item.id.as_deref(), fresh.id.as_deref()) {
        if expected != actual {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("entry identity changed before deletion: {}", item.path),
            ));
        }
    }
    if item.is_dir {
        if fresh.is_symlink || !fresh.is_dir {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("directory changed type before deletion: {}", item.path),
            ));
        }
        backend.remove_dir(&item.path)
    } else {
        if fresh.is_dir && !fresh.is_symlink {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "file changed into a directory before deletion: {}",
                    item.path
                ),
            ));
        }
        backend.remove_file_id(&item.path, item.id.as_deref().or(fresh.id.as_deref()))
    }
}

fn report_planning<F>(progress: &mut F, budget: &DeleteBudget, path: &str)
where
    F: FnMut(RecursiveDeleteProgress),
{
    progress(RecursiveDeleteProgress {
        phase: RecursiveDeletePhase::Planning,
        planned: budget.entries,
        removed: 0,
        current: path.to_string(),
    });
}

fn canceled_report(budget: &DeleteBudget, removed: u64) -> RecursiveDeleteReport {
    RecursiveDeleteReport {
        status: RecursiveDeleteStatus::Canceled,
        planned: budget.entries,
        removed,
    }
}

fn delete_failure(error: io::Error, budget: &DeleteBudget, removed: u64) -> RecursiveDeleteFailure {
    RecursiveDeleteFailure {
        error,
        planned: budget.entries,
        removed,
    }
}

pub(crate) fn validate_child_name(name: &str) -> VfsResult<()> {
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("backend returned unsafe child name: {name:?}"),
        ));
    }
    Ok(())
}

fn join_child(parent: &str, child: &str) -> String {
    let base = parent.trim_end_matches('/');
    if base.is_empty() {
        format!("/{child}")
    } else {
        format!("{base}/{child}")
    }
}
