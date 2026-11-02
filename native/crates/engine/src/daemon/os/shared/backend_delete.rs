use crate::vfs::{BackendHandle, VfsMeta};
use std::collections::HashSet;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_DELETE_NODES: u64 = 1_000_000;
const MAX_DELETE_TEXT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_DELETE_DEPTH: usize = 512;

#[derive(Clone)]
struct Candidate {
    path: String,
    metadata: VfsMeta,
}

#[derive(Default)]
struct DeleteBudget {
    nodes: u64,
    text_bytes: u64,
}

impl DeleteBudget {
    fn record(&mut self, path: &str, depth: usize) -> io::Result<()> {
        if depth > MAX_DELETE_DEPTH {
            return Err(invalid(format!(
                "backend deletion exceeds {MAX_DELETE_DEPTH} levels"
            )));
        }
        self.nodes = self.nodes.saturating_add(1);
        self.text_bytes = self.text_bytes.saturating_add(path.len() as u64);
        if self.nodes > MAX_DELETE_NODES || self.text_bytes > MAX_DELETE_TEXT_BYTES {
            return Err(invalid("backend deletion exceeds its bounded plan budget"));
        }
        Ok(())
    }
}

pub(super) fn remove_tree_backend(
    backend: &BackendHandle,
    path: &str,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let mut plan = Vec::new();
    collect(
        backend,
        path,
        0,
        cancel,
        &mut DeleteBudget::default(),
        &mut plan,
    )?;

    // Complete, read-only preflight before the first mutation.
    for candidate in &plan {
        check_cancel(cancel)?;
        let current = backend.stat(&candidate.path)?;
        if !same_candidate(&candidate.metadata, &current) {
            return Err(invalid(format!(
                "backend deletion target changed during preflight: {}",
                candidate.path
            )));
        }
    }

    for candidate in plan {
        check_cancel(cancel)?;
        let current = backend.stat(&candidate.path)?;
        if !same_candidate(&candidate.metadata, &current) {
            return Err(invalid(format!(
                "backend deletion target changed before removal: {}",
                candidate.path
            )));
        }
        if current.is_dir && !current.is_symlink {
            backend.remove_dir(&candidate.path)?;
        } else {
            backend.remove_file_id(
                &candidate.path,
                candidate.metadata.id.as_deref().or(current.id.as_deref()),
            )?;
        }
    }
    Ok(())
}

fn collect(
    backend: &BackendHandle,
    path: &str,
    depth: usize,
    cancel: &AtomicBool,
    budget: &mut DeleteBudget,
    plan: &mut Vec<Candidate>,
) -> io::Result<()> {
    check_cancel(cancel)?;
    budget.record(path, depth)?;
    let metadata = backend.stat(path)?;
    if metadata.is_dir && !metadata.is_symlink {
        let mut names = HashSet::new();
        for listed in backend.list_dir(path)? {
            check_cancel(cancel)?;
            crate::vfs::validate_child_name(&listed.name)?;
            if !names.insert(listed.name.clone()) {
                return Err(invalid(format!(
                    "backend returned duplicate child name in {path}: {:?}",
                    listed.name
                )));
            }
            let child_path = join(path, &listed.name);
            let child = backend.stat(&child_path)?;
            if !same_listing_type(&listed, &child) {
                return Err(invalid(format!(
                    "backend child changed type during deletion planning: {child_path}"
                )));
            }
            collect(backend, &child_path, depth + 1, cancel, budget, plan)?;
        }
    }
    plan.push(Candidate {
        path: path.into(),
        metadata,
    });
    Ok(())
}

fn same_listing_type(listed: &VfsMeta, current: &VfsMeta) -> bool {
    listed.is_dir == current.is_dir
        && listed.is_symlink == current.is_symlink
        && match (listed.id.as_deref(), current.id.as_deref()) {
            (Some(left), Some(right)) => left == right,
            _ => true,
        }
}

fn same_candidate(expected: &VfsMeta, current: &VfsMeta) -> bool {
    expected.is_dir == current.is_dir
        && expected.is_symlink == current.is_symlink
        && expected.id == current.id
        && (expected.is_dir
            || (expected.size == current.size
                && expected.mtime_ms == current.mtime_ms
                && expected.content_md5 == current.content_md5))
}

fn join(parent: &str, child: &str) -> String {
    if parent == "/" {
        format!("/{child}")
    } else {
        format!("{}/{child}", parent.trim_end_matches('/'))
    }
}

fn check_cancel(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "backend recursive deletion canceled",
        ))
    } else {
        Ok(())
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
#[path = "backend_delete_tests.rs"]
mod tests;
