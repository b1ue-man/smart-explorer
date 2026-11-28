use super::entry_kind::EntryKind;
use super::ops::parent_of;
use super::tree_guard::{
    inspect_destination, join, metadata_text_bytes, reject_link, validate_destination_state,
    validate_listed_names, validate_listing, validate_same_source, DestinationState,
};
use super::tree_observer::TreeObserver;
use crate::vfs::{Backend, OmissionReason, VfsMeta, VfsOmission};
use std::collections::HashMap;

#[path = "tree_plan_budget.rs"]
mod budget;
#[path = "tree_limits.rs"]
pub(super) mod limits;
use budget::PlanBudget;
use limits::TargetRules;

const MAX_TREE_DEPTH: usize = 512;

pub(super) struct TransferPlan {
    pub(super) entries: Vec<PlanEntry>,
    pub(super) destination_ancestors: Vec<DestinationAncestor>,
    /// Links and special files inside the tree (never followed, recreated or
    /// read), entries the destination cannot store and names the listing
    /// cannot represent. A move keeps them, and the directories holding
    /// them, in place.
    pub(super) omitted: Vec<OmittedEntry>,
}

pub(super) struct PlanEntry {
    pub(super) resume_existing: bool,
    pub(super) source_path: String,
    pub(super) destination_path: String,
    pub(super) source: VfsMeta,
    pub(super) destination: DestinationState,
    pub(super) parent: Option<usize>,
    pub(super) children: Vec<usize>,
    /// Indexes into [`TransferPlan::omitted`] of this directory's children.
    pub(super) omitted: Vec<usize>,
}

pub(super) struct OmittedEntry {
    pub(super) source_path: String,
    /// The listed entry; for `OmitCause::Unlisted` only its (lossy) name.
    pub(super) listed: VfsMeta,
    pub(super) kind: EntryKind,
    pub(super) cause: OmitCause,
}

/// Why a plan leaves an existing source entry out.
pub(super) enum OmitCause {
    /// A link or special file (`EntryKind::is_omitted_in_trees`).
    Kind,
    /// The destination cannot store it (name, size, or a name it treats as
    /// another planned one): the German reason.
    Refused(String),
    /// The source listing cannot name it (not valid Unicode, a name Win32
    /// cannot address): its kind is unknown (`kind` says `File`).
    Unlisted(String),
}

impl OmittedEntry {
    /// The German reason a refused or unlisted entry is reported with.
    pub(super) fn reason(&self) -> Option<String> {
        match &self.cause {
            OmitCause::Kind => None,
            OmitCause::Refused(reason) => Some(reason.clone()),
            OmitCause::Unlisted(detail) => Some(format!("Name nicht übertragbar ({detail})")),
        }
    }
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

impl TransferPlan {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn build(
        source: &dyn Backend,
        source_path: &str,
        destination: &dyn Backend,
        destination_path: &str,
        recursive: bool,
        force: bool,
        resume: bool,
        expected_root: Option<&VfsMeta>,
        observer: &TreeObserver,
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
        let mut omitted: Vec<OmittedEntry> = Vec::new();
        let rules = TargetRules::of(destination, destination_path);

        while let Some(work) = pending.pop() {
            observer.check()?;
            let fresh = source
                .stat(&work.source_path)
                .map_err(|error| format!("preflight stat {}: {error}", work.source_path))?;
            budget.account_fresh_metadata(&fresh, work.accounted_metadata_text)?;
            reject_link(&fresh, &work.source_path)?;
            if fresh.special && !fresh.is_dir {
                return Err(format!(
                    "{} is a special file (FIFO, socket or device) and has no content to copy",
                    work.source_path
                ));
            }
            if work.parent.is_none() {
                if let Some(expected) = expected_root {
                    validate_same_source(expected, &fresh, &work.source_path)?;
                }
                let name = leaf(&work.destination_path);
                if let Some(reason) = rules.refusal(name, &fresh) {
                    return Err(format!("{}: {reason}", work.destination_path));
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
            let destination_state = inspect_destination(
                destination,
                &work.destination_path,
                fresh.is_dir,
                force || resume,
            )?;
            budget.reserve_auxiliary_text(destination_state_text(&destination_state))?;
            let index = entries.len();
            entries.push(PlanEntry {
                resume_existing: resume
                    && !fresh.is_dir
                    && matches!(destination_state, DestinationState::File(_)),
                source_path: work.source_path,
                destination_path: work.destination_path,
                source: fresh.clone(),
                destination: destination_state,
                parent: work.parent,
                children: Vec::new(),
                omitted: Vec::new(),
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
            let listing = super::tree_guard::listing(source, &entries[index].source_path).map_err(
                |error| format!("preflight list {}: {error}", entries[index].source_path),
            )?;
            budget.ensure_entry_capacity(listing.entries.len() + listing.omitted.len())?;
            validate_listed_names(&entries[index].source_path, &listing.entries)?;
            for omission in listing.omitted {
                let Some(detail) = unlisted_detail(&entries[index].source_path, &omission)? else {
                    continue;
                };
                let child_source = join(&entries[index].source_path, &omission.rel);
                budget.reserve_auxiliary_path(&child_source)?;
                entries[index].omitted.push(omitted.len());
                omitted.push(OmittedEntry {
                    source_path: child_source,
                    listed: VfsMeta {
                        name: omission.rel,
                        ..VfsMeta::default()
                    },
                    kind: EntryKind::File,
                    cause: OmitCause::Unlisted(detail),
                });
            }
            let mut refused = rules.refusals(&listing.entries);
            for (position, child) in listing.entries.into_iter().enumerate().rev() {
                let child_source = join(&entries[index].source_path, &child.name);
                let kind = EntryKind::of(&child);
                let cause = match refused.remove(&position) {
                    Some(reason) => Some(OmitCause::Refused(reason)),
                    None if kind.is_omitted_in_trees() => Some(OmitCause::Kind),
                    None => None,
                };
                if let Some(cause) = cause {
                    budget.reserve_auxiliary_path(&child_source)?;
                    entries[index].omitted.push(omitted.len());
                    omitted.push(OmittedEntry {
                        source_path: child_source,
                        listed: child,
                        kind,
                        cause,
                    });
                    continue;
                }
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
            omitted,
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
            let listing =
                super::tree_guard::listing(backend, &entry.source_path).map_err(|error| {
                    format!(
                        "source tree changed before transfer: {}: {error}",
                        entry.source_path
                    )
                })?;
            if listing.entries.len() + listing.omitted.len()
                != entry.children.len() + entry.omitted.len()
            {
                return Err(format!(
                    "source tree child count changed before transfer: {}",
                    entry.source_path
                ));
            }
            validate_listed_names(&entry.source_path, &listing.entries)?;
            let mut expected: HashMap<&str, usize> = entry
                .children
                .iter()
                .map(|child| (self.entries[*child].source.name.as_str(), *child))
                .collect();
            let mut skipped: HashMap<&str, &OmittedEntry> = entry
                .omitted
                .iter()
                .map(|index| {
                    let omission = &self.omitted[*index];
                    (omission.listed.name.as_str(), omission)
                })
                .collect();
            for omission in &listing.omitted {
                if !skipped
                    .remove(omission.rel.as_str())
                    .is_some_and(|planned| matches!(planned.cause, OmitCause::Unlisted(_)))
                {
                    return Err(format!(
                        "source tree changed before transfer: {}/{}: {}",
                        entry.source_path, omission.rel, omission.detail
                    ));
                }
            }
            for child in listing.entries {
                if let Some(omission) = skipped.remove(child.name.as_str()) {
                    if EntryKind::of(&child) != omission.kind
                        || matches!(omission.cause, OmitCause::Unlisted(_))
                    {
                        return Err(format!(
                            "source changed type before transfer: {}",
                            omission.source_path
                        ));
                    }
                    continue;
                }
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

/// The detail of an entry a tolerant listing could not name, which the plan
/// leaves out and reports; `None` for an entry that vanished meanwhile (it
/// is no child any more). An unreadable entry fails the plan as before.
fn unlisted_detail(directory: &str, omission: &VfsOmission) -> Result<Option<String>, String> {
    match omission.reason {
        OmissionReason::Vanished => Ok(None),
        OmissionReason::Unrepresentable => Ok(Some(omission.detail.clone())),
        _ => Err(format!(
            "preflight list {directory}: {}: {}",
            omission.rel, omission.detail
        )),
    }
}

fn leaf(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or("")
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
