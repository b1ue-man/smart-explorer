use super::ops::parent_of;
use super::tree_guard::{
    inspect_destination, join, metadata_text_bytes, reject_link, validate_destination_state,
    validate_listed_names, validate_listing, validate_same_source, DestinationState,
};
use crate::vfs::{Backend, VfsMeta};
use std::collections::HashMap;

const MAX_TREE_ENTRIES: u64 = 1_000_000;
const MAX_TREE_TEXT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_TREE_DEPTH: usize = 512;

pub(super) struct TransferPlan {
    pub(super) entries: Vec<PlanEntry>,
    pub(super) destination_ancestors: Vec<DestinationAncestor>,
}

pub(super) struct PlanEntry {
    pub(super) source_path: String,
    pub(super) destination_path: String,
    pub(super) source: VfsMeta,
    pub(super) destination: DestinationState,
    pub(super) parent: Option<usize>,
    pub(super) children: Vec<usize>,
}

pub(super) struct DestinationAncestor {
    pub(super) path: String,
    pub(super) state: DestinationState,
}

struct PendingEntry {
    source_path: String,
    destination_path: String,
    parent: Option<usize>,
    depth: usize,
    listed: Option<VfsMeta>,
    accounted_metadata_text: u64,
}

#[derive(Default)]
struct PlanBudget {
    entries: u64,
    text_bytes: u64,
}

impl PlanBudget {
    fn reserve_entry(
        &mut self,
        source_path: &str,
        destination_path: &str,
        listed: Option<&VfsMeta>,
        depth: usize,
    ) -> Result<u64, String> {
        if depth > MAX_TREE_DEPTH {
            return Err(format!("transfer tree exceeds {MAX_TREE_DEPTH} levels"));
        }
        self.entries = self
            .entries
            .checked_add(1)
            .ok_or_else(|| "transfer entry count overflow".to_string())?;
        let metadata_text = listed.map(metadata_text_bytes).unwrap_or(0);
        self.add_text(
            (source_path.len() as u64)
                .checked_add(destination_path.len() as u64)
                .and_then(|value| value.checked_add(metadata_text))
                .ok_or_else(|| "transfer path text budget overflow".to_string())?,
        )?;
        self.check()?;
        Ok(metadata_text)
    }

    fn account_fresh_metadata(&mut self, fresh: &VfsMeta, accounted: u64) -> Result<(), String> {
        let fresh_text = metadata_text_bytes(fresh);
        if fresh_text > accounted {
            self.add_text(fresh_text - accounted)?;
        }
        self.check()
    }

    fn reserve_auxiliary_path(&mut self, path: &str) -> Result<(), String> {
        self.add_text(path.len() as u64)?;
        self.check()
    }

    fn reserve_auxiliary_text(&mut self, bytes: u64) -> Result<(), String> {
        self.add_text(bytes)?;
        self.check()
    }

    fn ensure_entry_capacity(&self, additional: usize) -> Result<(), String> {
        let additional = u64::try_from(additional)
            .map_err(|_| "transfer child count does not fit its budget".to_string())?;
        if self
            .entries
            .checked_add(additional)
            .is_none_or(|entries| entries > MAX_TREE_ENTRIES)
        {
            Err("transfer tree exceeds its bounded collection budget".to_string())
        } else {
            Ok(())
        }
    }

    fn add_text(&mut self, bytes: u64) -> Result<(), String> {
        self.text_bytes = self
            .text_bytes
            .checked_add(bytes)
            .ok_or_else(|| "transfer path text budget overflow".to_string())?;
        Ok(())
    }

    fn check(&self) -> Result<(), String> {
        if self.entries > MAX_TREE_ENTRIES || self.text_bytes > MAX_TREE_TEXT_BYTES {
            Err("transfer tree exceeds its bounded collection budget".to_string())
        } else {
            Ok(())
        }
    }
}

impl TransferPlan {
    pub(super) fn build(
        source: &dyn Backend,
        source_path: &str,
        destination: &dyn Backend,
        destination_path: &str,
        recursive: bool,
        force: bool,
        expected_root: Option<&VfsMeta>,
    ) -> Result<Self, String> {
        let mut budget = PlanBudget::default();
        let root_accounted =
            budget.reserve_entry(source_path, destination_path, expected_root, 0)?;
        let mut pending = vec![PendingEntry {
            source_path: source_path.to_string(),
            destination_path: destination_path.to_string(),
            parent: None,
            depth: 0,
            listed: None,
            accounted_metadata_text: root_accounted,
        }];
        let mut entries: Vec<PlanEntry> = Vec::new();

        while let Some(work) = pending.pop() {
            let fresh = source
                .stat(&work.source_path)
                .map_err(|error| format!("preflight stat {}: {error}", work.source_path))?;
            budget.account_fresh_metadata(&fresh, work.accounted_metadata_text)?;
            reject_link(&fresh, &work.source_path)?;
            if work.parent.is_none() {
                if let Some(expected) = expected_root {
                    validate_same_source(expected, &fresh, &work.source_path)?;
                }
            }
            if let Some(listed) = work.listed.as_ref() {
                validate_listing(listed, &fresh, &work.source_path)?;
            }
            if fresh.is_dir && !recursive {
                return Err(format!(
                    "{} is a directory; pass --recursive",
                    work.source_path
                ));
            }
            let destination_state =
                inspect_destination(destination, &work.destination_path, fresh.is_dir, force)?;
            budget.reserve_auxiliary_text(destination_state_text(&destination_state))?;
            let index = entries.len();
            entries.push(PlanEntry {
                source_path: work.source_path,
                destination_path: work.destination_path,
                source: fresh.clone(),
                destination: destination_state,
                parent: work.parent,
                children: Vec::new(),
            });
            if let Some(parent) = work.parent {
                entries[parent].children.push(index);
            }
            if !fresh.is_dir {
                continue;
            }

            validate_same_source(
                &fresh,
                &source.stat(&entries[index].source_path).map_err(|error| {
                    format!(
                        "preflight directory stat {}: {error}",
                        entries[index].source_path
                    )
                })?,
                &entries[index].source_path,
            )?;
            let listed = source
                .list_dir(&entries[index].source_path)
                .map_err(|error| {
                    format!("preflight list {}: {error}", entries[index].source_path)
                })?;
            budget.ensure_entry_capacity(listed.len())?;
            validate_listed_names(&entries[index].source_path, &listed)?;
            for child in listed.into_iter().rev() {
                reject_link(&child, &join(&entries[index].source_path, &child.name))?;
                let child_source = join(&entries[index].source_path, &child.name);
                let child_destination = join(&entries[index].destination_path, &child.name);
                let accounted = budget.reserve_entry(
                    &child_source,
                    &child_destination,
                    Some(&child),
                    work.depth + 1,
                )?;
                pending.push(PendingEntry {
                    source_path: child_source,
                    destination_path: child_destination,
                    parent: Some(index),
                    depth: work.depth + 1,
                    listed: Some(child),
                    accounted_metadata_text: accounted,
                });
            }
            validate_same_source(
                &fresh,
                &source.stat(&entries[index].source_path).map_err(|error| {
                    format!(
                        "preflight directory changed while listing {}: {error}",
                        entries[index].source_path
                    )
                })?,
                &entries[index].source_path,
            )?;
        }

        let root_destination = entries
            .first()
            .ok_or_else(|| "transfer plan unexpectedly contains no root".to_string())?
            .destination_path
            .clone();
        let destination_ancestors =
            collect_destination_ancestors(destination, &root_destination, &mut budget)?;
        let plan = Self {
            entries,
            destination_ancestors,
        };
        plan.validate_source_tree(source)?;
        plan.validate_destinations(destination)?;
        Ok(plan)
    }

    pub(super) fn validate_source_tree(&self, backend: &dyn Backend) -> Result<(), String> {
        for entry in &self.entries {
            let current = backend.stat(&entry.source_path).map_err(|error| {
                format!(
                    "source changed before transfer: {}: {error}",
                    entry.source_path
                )
            })?;
            validate_same_source(&entry.source, &current, &entry.source_path)?;
            if !entry.source.is_dir {
                continue;
            }
            let listed = backend.list_dir(&entry.source_path).map_err(|error| {
                format!(
                    "source tree changed before transfer: {}: {error}",
                    entry.source_path
                )
            })?;
            if listed.len() != entry.children.len() {
                return Err(format!(
                    "source tree child count changed before transfer: {}",
                    entry.source_path
                ));
            }
            validate_listed_names(&entry.source_path, &listed)?;
            let mut expected: HashMap<&str, usize> = entry
                .children
                .iter()
                .map(|child| (self.entries[*child].source.name.as_str(), *child))
                .collect();
            for child in listed {
                reject_link(&child, &join(&entry.source_path, &child.name))?;
                let child_index = expected.remove(child.name.as_str()).ok_or_else(|| {
                    format!(
                        "source tree gained an unplanned child: {}/{}",
                        entry.source_path, child.name
                    )
                })?;
                let planned = &self.entries[child_index];
                validate_listing(&child, &planned.source, &planned.source_path)?;
                let fresh = backend.stat(&planned.source_path).map_err(|error| {
                    format!(
                        "source child changed before transfer: {}: {error}",
                        planned.source_path
                    )
                })?;
                validate_same_source(&planned.source, &fresh, &planned.source_path)?;
            }
            if let Some((missing, _)) = expected.into_iter().next() {
                return Err(format!(
                    "source tree lost a planned child: {}/{}",
                    entry.source_path, missing
                ));
            }
            validate_same_source(
                &entry.source,
                &backend.stat(&entry.source_path).map_err(|error| {
                    format!(
                        "source directory changed while validating: {}: {error}",
                        entry.source_path
                    )
                })?,
                &entry.source_path,
            )?;
        }
        Ok(())
    }

    pub(super) fn validate_source_entry(
        &self,
        backend: &dyn Backend,
        index: usize,
    ) -> Result<(), String> {
        let mut chain = Vec::new();
        let mut current = Some(index);
        while let Some(candidate) = current {
            chain.push(candidate);
            current = self.entries[candidate].parent;
        }
        for candidate in chain.into_iter().rev() {
            let entry = &self.entries[candidate];
            let fresh = backend.stat(&entry.source_path).map_err(|error| {
                format!(
                    "source changed immediately before transfer: {}: {error}",
                    entry.source_path
                )
            })?;
            validate_same_source(&entry.source, &fresh, &entry.source_path)?;
        }
        Ok(())
    }

    pub(super) fn validate_destinations(&self, backend: &dyn Backend) -> Result<(), String> {
        for ancestor in &self.destination_ancestors {
            validate_destination_state(backend, &ancestor.path, &ancestor.state, true)?;
        }
        for entry in &self.entries {
            validate_destination_state(
                backend,
                &entry.destination_path,
                &entry.destination,
                entry.source.is_dir,
            )?;
        }
        super::tree_destination::validate_all_collisions(self, backend)
    }

    pub(super) fn validate_destination_ancestry(
        &self,
        backend: &dyn Backend,
        index: usize,
    ) -> Result<(), String> {
        for ancestor in &self.destination_ancestors {
            validate_destination_state(backend, &ancestor.path, &ancestor.state, true)?;
        }
        let mut parents = Vec::new();
        let mut current = self.entries[index].parent;
        while let Some(parent) = current {
            parents.push(parent);
            current = self.entries[parent].parent;
        }
        for parent in parents.into_iter().rev() {
            let entry = &self.entries[parent];
            validate_destination_state(backend, &entry.destination_path, &entry.destination, true)?;
        }
        Ok(())
    }

    pub(super) fn validate_destination_parent_collision(
        &self,
        backend: &dyn Backend,
        index: usize,
    ) -> Result<(), String> {
        super::tree_destination::validate_parent_collision(self, backend, index)
    }
}

fn collect_destination_ancestors(
    backend: &dyn Backend,
    destination_path: &str,
    budget: &mut PlanBudget,
) -> Result<Vec<DestinationAncestor>, String> {
    let mut ancestors = Vec::new();
    let mut current = parent_of(destination_path);
    while let Some(path) = current {
        if ancestors.len() >= MAX_TREE_DEPTH {
            return Err(format!(
                "destination ancestry exceeds {MAX_TREE_DEPTH} levels"
            ));
        }
        budget.reserve_auxiliary_path(&path)?;
        let state = inspect_destination(backend, &path, true, false)?;
        budget.reserve_auxiliary_text(destination_state_text(&state))?;
        current = parent_of(&path).filter(|parent| parent != &path);
        ancestors.push(DestinationAncestor { path, state });
    }
    ancestors.reverse();
    Ok(ancestors)
}

fn destination_state_text(state: &DestinationState) -> u64 {
    match state {
        DestinationState::Missing => 0,
        DestinationState::Directory(metadata) | DestinationState::File(metadata) => {
            metadata_text_bytes(metadata)
        }
    }
}
