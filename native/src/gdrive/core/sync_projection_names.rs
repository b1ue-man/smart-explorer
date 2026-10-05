//! Pure stable folder-key allocation and exact picker aliases. Existing
//! identities and logical names are never reassigned by new sibling ordering.
use super::identity::invalid;
use super::listing::RawEntry;
use super::sync_bindings::{FolderBinding, FolderBindings};
use super::sync_projection::representable;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;

pub(super) fn allocate_folders(
    record: &mut FolderBindings,
    raw: &[RawEntry],
    hints: &[FolderBinding],
) -> io::Result<HashMap<String, FolderBinding>> {
    // Valid old plain/short-marker cache locators are stronger evidence than
    // a new mtime ordering. Import them atomically before allocating new IDs.
    for hint in hints {
        // Equal reserved IDs gain this exact proved segment. Conflicting IDs
        // remain reserved and make the transaction fail without replacement.
        record.bind_exact(&hint.title, &hint.id, &hint.segment, &hint.sync_name, true)?;
    }
    let literal_names: HashSet<String> = raw.iter().map(|entry| entry.meta.name.clone()).collect();
    let mut literal_segments: HashSet<String> = literal_names
        .iter()
        .map(|name| super::names::encode(name))
        .collect();
    literal_segments.extend(
        super::duplicates::disambiguate(
            raw.iter()
                .filter(|entry| !entry.meta.is_dir)
                .map(|entry| entry.meta.clone())
                .collect(),
        )
        .into_iter()
        .map(|entry| entry.name),
    );
    let file_names: HashSet<&str> = raw
        .iter()
        .filter(|entry| !entry.meta.is_dir)
        .map(|entry| entry.meta.name.as_str())
        .collect();
    let mut groups: BTreeMap<&str, Vec<&RawEntry>> = BTreeMap::new();
    for entry in raw
        .iter()
        .filter(|entry| entry.meta.is_dir && representable(&entry.meta.name))
    {
        groups.entry(&entry.meta.name).or_default().push(entry);
    }
    let mut result: HashMap<String, FolderBinding> = raw
        .iter()
        .filter(|entry| entry.meta.is_dir)
        .filter_map(|entry| {
            let id = entry.meta.id.as_deref()?;
            record
                .by_object(&entry.meta.name, id)
                .cloned()
                .map(|binding| (id.to_string(), binding))
        })
        .collect();
    for (title, mut group) in groups {
        group.sort_by(|left, right| {
            right
                .meta
                .mtime_ms
                .cmp(&left.meta.mtime_ms)
                .then_with(|| left.meta.id.cmp(&right.meta.id))
        });
        for entry in &group {
            let id = entry
                .meta
                .id
                .as_deref()
                .ok_or_else(|| invalid("Drive folder has no ID"))?;
            if let Some(existing) = record.by_object(title, id).cloned() {
                let binding =
                    picker_binding(record, existing, &group, &literal_names, &literal_segments)?;
                result.insert(id.to_string(), binding);
                continue;
            }
            let plain_segment = super::names::encode(title);
            let plain = !file_names.contains(title)
                && record.by_name(title).is_none()
                && record.by_segment(&plain_segment).is_none();
            let (name, segment) = if plain {
                // The logical plain key is still canonical. For an unknown
                // ambiguous group, the picker offers an exact ID locator for
                // that folder too, so selecting it is real new evidence.
                let segment = if group.len() > 1 {
                    available_marker(record, title, id, &group, &literal_names, &literal_segments).1
                } else {
                    plain_segment
                };
                (title.to_string(), segment)
            } else {
                available_marker(record, title, id, &group, &literal_names, &literal_segments)
            };
            let binding = record.bind_exact(title, id, &segment, &name, false)?;
            result.insert(id.to_string(), binding);
        }
    }
    Ok(result)
}

fn picker_binding(
    record: &mut FolderBindings,
    binding: FolderBinding,
    group: &[&RawEntry],
    literals: &HashSet<String>,
    literal_segments: &HashSet<String>,
) -> io::Result<FolderBinding> {
    if group.len() < 2
        || binding.proves(&binding.segment)
        || super::duplicates::parse_marker(&binding.segment).is_some()
    {
        return Ok(binding);
    }
    let segment = binding
        .aliases
        .iter()
        .find(|segment| super::duplicates::parse_marker(segment).is_some())
        .cloned()
        .unwrap_or_else(|| {
            available_marker(
                record,
                &binding.title,
                &binding.id,
                group,
                literals,
                literal_segments,
            )
            .1
        });
    let mut exact = record.bind_exact(
        &binding.title,
        &binding.id,
        &segment,
        &binding.sync_name,
        false,
    )?;
    // Keep the durable primary segment and logical key unchanged. The browser
    // may display this registered exact alias without redefining either one.
    exact.segment = segment;
    Ok(exact)
}

fn available_marker(
    record: &FolderBindings,
    title: &str,
    id: &str,
    group: &[&RawEntry],
    literals: &HashSet<String>,
    literal_segments: &HashSet<String>,
) -> (String, String) {
    let prefix: String = id.chars().take(super::duplicates::ID_PREFIX_LEN).collect();
    let prefix_unique = group
        .iter()
        .filter(|entry| {
            entry
                .meta
                .id
                .as_deref()
                .is_some_and(|id| id.starts_with(&prefix))
        })
        .count()
        == 1
        && !record.folders.iter().any(|folder| {
            folder.title == title && folder.id != id && folder.id.starts_with(&prefix)
        });
    let marker = |prefix: &str| {
        format!(
            "{}{}{}",
            super::duplicates::MARKER_PREFIX,
            prefix,
            super::duplicates::MARKER_SUFFIX
        )
    };
    let free_name = |name: &str| {
        !literals
            .iter()
            .any(|literal| portable_key(literal) == portable_key(name))
            && !record
                .folders
                .iter()
                .any(|folder| portable_key(&folder.sync_name) == portable_key(name))
    };
    let free_segment =
        |segment: &str| !literal_segments.contains(segment) && record.by_segment(segment).is_none();
    let short = marker(&prefix);
    let name = format!("{title}{short}");
    let segment = format!("{}{short}", super::names::encode(title));
    if prefix_unique && free_name(&name) && free_segment(&segment) {
        return (name, segment);
    }
    let full = marker(id);
    let mut name = format!("{title}{full}");
    let mut segment = format!("{}{full}", super::names::encode(title));
    // Locator identity and the portable logical key are separate. Distinct
    // Drive IDs can differ only by ASCII case, so even full ID markers can
    // collide on a case-insensitive target. Their complete byte encoding is
    // deterministic, collision-free and needs no new locator interpretation.
    if !free_name(&name) {
        let hex: String = id.bytes().map(|byte| format!("{byte:02x}")).collect();
        let suffix = marker(&format!("{id}-{hex}"));
        name = format!("{title}{suffix}");
        while !free_name(&name) {
            name.push_str(&suffix);
        }
    }
    // Each collision is a finite existing name. Prior aliases, including
    // short markers whose prefix later collides, stay unchanged.
    while !free_segment(&segment) {
        segment.push_str(&full);
    }
    (name, segment)
}

fn portable_key(name: &str) -> String {
    name.to_uppercase()
}
