//! One folder of the size tree as its entries arrive, shared by the
//! directory walk and the MFT reader: the largest files keep an own node,
//! the rest of a huge folder and every subtree beyond the retention budget
//! fold into one aggregate, and size and occupied bytes stay exact.
use super::MAX_RETAINED_FILES_PER_DIRECTORY;
use super::{AnalyticsBudget, Diagnostics, Progress, Retention, SizeNode};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::{cmp::Reverse, collections::BinaryHeap};

/// Logical and occupied bytes counted together.
#[derive(Clone, Copy, Default)]
pub(super) struct Bytes {
    pub(super) size: u64,
    pub(super) allocated: u64,
}

impl Bytes {
    pub(super) fn add(&mut self, size: u64, allocated: u64) {
        self.size = self.size.saturating_add(size);
        self.allocated = self.allocated.saturating_add(allocated);
    }
}

/// Files counted between two progress reports.
const REPORT_FILES: u64 = 128;

#[derive(Default)]
pub(super) struct Assembly {
    /// The weakest retained file is on top: lower size, then later name.
    /// The enumeration order is unique, so the occupied bytes never decide.
    files: BinaryHeap<(Reverse<u64>, Box<str>, u64, u64)>,
    own_files: u64,
    own: Bytes,
    aggregated: Bytes,
    aggregated_entries: u64,
    subtree: Bytes,
    dir_nodes: Vec<SizeNode>,
    reported_files: u64,
    reported: Bytes,
}

impl Assembly {
    /// A file of this folder; its name is copied only when it is retained.
    pub(super) fn file(&mut self, name: &str, size: u64, allocated: u64) {
        self.own_files += 1;
        self.own.add(size, allocated);
        let keep = self.files.len() < MAX_RETAINED_FILES_PER_DIRECTORY
            || self
                .files
                .peek()
                .is_some_and(|(weakest, weakest_name, _, _)| {
                    size > weakest.0 || (size == weakest.0 && name < weakest_name.as_ref())
                });
        if keep {
            if self.files.len() == MAX_RETAINED_FILES_PER_DIRECTORY {
                if let Some((size, _, _, weakest)) = self.files.pop() {
                    self.aggregated.add(size.0, weakest);
                    self.aggregated_entries += 1;
                }
            }
            let order = self.own_files;
            self.files
                .push((Reverse(size), name.into(), order, allocated));
        } else {
            self.aggregated.add(size, allocated);
            self.aggregated_entries += 1;
        }
    }

    /// A finished subfolder with the retention its claim got.
    pub(super) fn child(&mut self, node: SizeNode, retention: Retention) {
        self.subtree.add(node.size, node.allocated);
        match retention {
            Retention::Keep => self.dir_nodes.push(node),
            Retention::Aggregate => {
                self.aggregated.add(node.size, node.allocated);
                self.aggregated_entries += 1;
            }
        }
    }

    /// Report the files counted since the last report, every `REPORT_FILES`
    /// files or with `force`.
    pub(super) fn report(&mut self, progress: &Progress, force: bool) {
        let files = self.own_files - self.reported_files;
        if files == 0 || (!force && files < REPORT_FILES) {
            return;
        }
        progress.files.fetch_add(files, Ordering::Relaxed);
        progress
            .bytes
            .fetch_add(self.own.size - self.reported.size, Ordering::Relaxed);
        progress.allocated.fetch_add(
            self.own.allocated - self.reported.allocated,
            Ordering::Relaxed,
        );
        self.reported_files = self.own_files;
        self.reported = self.own;
    }

    /// The folder node: retained files claim their budget (the largest first
    /// in a huge folder, else in listing order), the rest joins the
    /// aggregate.
    pub(super) fn finish(
        mut self,
        name: Box<str>,
        dir: &Path,
        depth: u32,
        budget: &AnalyticsBudget,
        diagnostics: &Diagnostics,
    ) -> SizeNode {
        let mut files = std::mem::take(&mut self.files).into_vec();
        if self.own_files > MAX_RETAINED_FILES_PER_DIRECTORY as u64 {
            files.sort_by(|left, right| {
                right
                    .0
                     .0
                    .cmp(&left.0 .0)
                    .then_with(|| left.1.cmp(&right.1))
            });
        } else {
            files.sort_by_key(|file| file.2);
        }
        let mut file_nodes = Vec::with_capacity(files.len());
        for (Reverse(size), file_name, _, allocated) in files {
            let retained = budget.claim(
                &dir.join(&*file_name),
                depth.saturating_add(1),
                file_name.len() as u64,
                diagnostics,
            ) == Retention::Keep;
            if retained {
                file_nodes.push(SizeNode {
                    name: file_name,
                    size,
                    allocated,
                    is_dir: false,
                    children: Vec::new(),
                });
            } else {
                self.aggregated.add(size, allocated);
                self.aggregated_entries += 1;
            }
        }
        diagnostics.count_aggregated_files(self.own_files.saturating_sub(file_nodes.len() as u64));
        let mut children = Vec::with_capacity(self.dir_nodes.len() + file_nodes.len() + 1);
        children.append(&mut self.dir_nodes);
        children.append(&mut file_nodes);
        if self.aggregated_entries > 0 {
            children.push(SizeNode {
                name: crate::analytics::aggregate_name(self.aggregated_entries).into_boxed_str(),
                size: self.aggregated.size,
                allocated: self.aggregated.allocated,
                is_dir: false,
                children: Vec::new(),
            });
        }
        SizeNode {
            name,
            size: self.own.size.saturating_add(self.subtree.size),
            allocated: self.own.allocated.saturating_add(self.subtree.allocated),
            is_dir: true,
            children,
        }
    }
}
