use super::ops::parent_of;
use super::tree_guard::DestinationState;
use super::tree_plan::TransferPlan;
use crate::vfs::{Backend, VfsMeta};
use std::collections::HashMap;

pub(super) fn validate_parent_collision(
    plan: &TransferPlan,
    backend: &dyn Backend,
    index: usize,
) -> Result<(), String> {
    let entry = &plan.entries[index];
    let name = leaf_name(&entry.destination_path)?;
    let parent = if let Some(parent) = entry.parent {
        plan.entries[parent].destination_path.clone()
    } else if let Some(parent) = inspectable_parent(backend, &entry.destination_path) {
        parent
    } else {
        return Ok(());
    };
    validate_directory_entries(
        backend,
        &parent,
        std::iter::once((name, &entry.destination)),
    )
}

pub(super) fn validate_all_collisions(
    plan: &TransferPlan,
    backend: &dyn Backend,
) -> Result<(), String> {
    for index in 0..plan.destination_ancestors.len() {
        if matches!(
            &plan.destination_ancestors[index].state,
            DestinationState::Directory(_)
        ) {
            validate_ancestor_collision(plan, backend, index)?;
        }
    }
    let root = plan
        .entries
        .first()
        .ok_or_else(|| "transfer plan unexpectedly contains no root".to_string())?;
    if let Some(parent) = inspectable_parent(backend, &root.destination_path) {
        let parent_exists = plan
            .destination_ancestors
            .iter()
            .find(|ancestor| ancestor.path == parent)
            .is_some_and(|ancestor| matches!(&ancestor.state, DestinationState::Directory(_)))
            || (parent == "." && backend.is_local());
        if parent_exists {
            validate_directory_entries(
                backend,
                &parent,
                std::iter::once((leaf_name(&root.destination_path)?, &root.destination)),
            )?;
        }
    }
    for directory in plan.entries.iter().filter(|entry| {
        entry.source.is_dir && matches!(&entry.destination, DestinationState::Directory(_))
    }) {
        validate_directory_entries(
            backend,
            &directory.destination_path,
            directory.children.iter().map(|child| {
                let entry = &plan.entries[*child];
                (entry.source.name.as_str(), &entry.destination)
            }),
        )?;
    }
    Ok(())
}

pub(super) fn validate_ancestor_collision(
    plan: &TransferPlan,
    backend: &dyn Backend,
    index: usize,
) -> Result<(), String> {
    let ancestor = &plan.destination_ancestors[index];
    let Some(parent) = inspectable_parent(backend, &ancestor.path) else {
        return Ok(());
    };
    let Ok(name) = leaf_name(&ancestor.path) else {
        return Ok(());
    };
    validate_directory_entries(backend, &parent, std::iter::once((name, &ancestor.state)))
}

fn validate_directory_entries<'a>(
    backend: &dyn Backend,
    directory: &str,
    expected: impl IntoIterator<Item = (&'a str, &'a DestinationState)>,
) -> Result<(), String> {
    let mut expected_names = HashMap::new();
    for (requested_name, state) in expected {
        let name = reported_name(requested_name, state);
        if expected_names
            .insert(name, (requested_name, state, false))
            .is_some()
        {
            return Err(format!(
                "multiple planned destination paths resolve to the same child in {directory}: {name:?}"
            ));
        }
    }
    let listed = backend
        .list_dir(directory)
        .map_err(|error| format!("cannot inspect destination directory {directory}: {error}"))?;
    for child in &listed {
        crate::vfs::validate_child_name(&child.name).map_err(|error| error.to_string())?;
        let Some((_, state, seen)) = expected_names.get_mut(child.name.as_str()) else {
            continue;
        };
        if *seen {
            return Err(format!(
                "destination contains a duplicate planned child in {directory}: {:?}",
                child.name
            ));
        }
        let state = *state;
        if matches!(state, DestinationState::Missing) {
            return Err(format!(
                "destination child appeared after preflight: {directory}/{}",
                child.name
            ));
        }
        validate_destination_listing(state, child, directory)?;
        *seen = true;
    }
    for (_, (requested_name, state, seen)) in expected_names {
        if !seen && !matches!(state, DestinationState::Missing) {
            return Err(format!(
                "destination child disappeared after preflight: {directory}/{requested_name}"
            ));
        }
    }
    Ok(())
}

fn reported_name<'a>(requested_name: &'a str, state: &'a DestinationState) -> &'a str {
    match state {
        DestinationState::Directory(metadata) | DestinationState::File(metadata) => &metadata.name,
        DestinationState::Missing => requested_name,
    }
}

fn validate_destination_listing(
    expected: &DestinationState,
    listed: &VfsMeta,
    directory: &str,
) -> Result<(), String> {
    let (metadata, is_dir) = match expected {
        DestinationState::Directory(metadata) => (metadata, true),
        DestinationState::File(metadata) => (metadata, false),
        DestinationState::Missing => return Ok(()),
    };
    let identity_changed = metadata.id != listed.id;
    if listed.is_symlink || listed.is_dir != is_dir || identity_changed {
        Err(format!(
            "destination listing changed type or identity in {directory}: {:?}",
            listed.name
        ))
    } else {
        Ok(())
    }
}

fn inspectable_parent(backend: &dyn Backend, path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty()
        || trimmed == "."
        || (trimmed.len() == 2 && trimmed.as_bytes()[1] == b':')
        || trimmed
            .strip_prefix("//")
            .is_some_and(|rest| rest.split('/').filter(|part| !part.is_empty()).count() <= 2)
    {
        return None;
    }
    parent_of(path).or_else(|| backend.is_local().then(|| ".".to_string()))
}

fn leaf_name(path: &str) -> Result<&str, String> {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("destination has no safe leaf name: {path}"))
}

#[cfg(test)]
mod tests {
    use super::{reported_name, DestinationState};
    use crate::vfs::VfsMeta;

    #[test]
    fn existing_destinations_match_the_name_reported_by_the_backend() {
        let existing = DestinationState::Directory(VfsMeta {
            name: "runneradmin".to_string(),
            is_dir: true,
            ..VfsMeta::default()
        });

        assert_eq!(reported_name("RUNNER~1", &existing), "runneradmin");
        assert_eq!(
            reported_name("RUNNER~1", &DestinationState::Missing),
            "RUNNER~1"
        );
    }
}
