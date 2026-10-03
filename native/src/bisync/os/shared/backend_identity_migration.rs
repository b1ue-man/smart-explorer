//! One-time state import under current and every proven previous pair lock.
use super::backend_identity_state::{self as state, Aliases, Binding};
use super::incremental::SyncEndpoints;
use super::pair_lock::PairLock;
use std::collections::BTreeSet;
use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::AtomicBool;

/// Retain these guards through the run/preview so an older identity cannot
/// concurrently mutate the same endpoint after its state was imported.
pub(super) fn migrate(
    lock: &PairLock,
    endpoints: SyncEndpoints<'_>,
    cancel: &AtomicBool,
) -> io::Result<Vec<PairLock>> {
    let current = Binding::current(endpoints);
    if lock.id() != current.lock {
        return Err(invalid("identity migration has another lock"));
    }
    let mut previous_a = crate::vfs::previous_state_identities(endpoints.a)?;
    let mut previous_b = crate::vfs::previous_state_identities(endpoints.b)?;
    previous_a.retain(|id| id != &current.identities[0]);
    previous_b.retain(|id| id != &current.identities[1]);
    let stored = state::read(&current.pair)?;
    if stored
        .as_ref()
        .is_some_and(|saved| saved.current != current)
    {
        return Err(invalid("identity state belongs to another endpoint"));
    }
    let mut previous = stored
        .as_ref()
        .map_or_else(Vec::new, |saved| saved.previous.clone());
    for a in std::iter::once(&current.identities[0]).chain(&previous_a) {
        for b in std::iter::once(&current.identities[1]).chain(&previous_b) {
            let binding = Binding::new([a.clone(), b.clone()], current.roots.clone());
            if stored.as_ref().is_some_and(|saved| saved.complete)
                && !previous.contains(&binding)
                && binding.pair != current.pair
                && !has_state(&super::replica_state::pair_dir(&binding.pair), None)?
                && !has_legacy_state(&binding.pair)?
                && !super::persistence::versions_dir(&binding.pair).try_exists()?
            {
                continue;
            }
            if binding.pair != current.pair && !previous.contains(&binding) {
                previous.push(binding);
            }
        }
    }
    let reversed = reverse(&current);
    let reversed_stored = state::read(&reversed.pair)?;
    if reversed_stored
        .as_ref()
        .is_some_and(|saved| saved.current != reversed)
    {
        return Err(invalid(
            "reversed identity state belongs to another endpoint",
        ));
    }
    if let Some(saved) = &reversed_stored {
        for old in &saved.previous {
            let old = reverse(old);
            if !previous.contains(&old) {
                previous.push(old);
            }
        }
    }
    if previous.len() > 80 {
        return Err(invalid("too many proven identity aliases"));
    }
    for side in 0..2 {
        let identities: BTreeSet<_> = previous
            .iter()
            .map(|binding| &binding.identities[side])
            .filter(|identity| *identity != &current.identities[side])
            .collect();
        if identities.len() > 8 {
            return Err(invalid("too many proven previous backend identities"));
        }
    }
    if previous.is_empty() {
        return Ok(Vec::new());
    }
    let mut guards = Vec::new();
    let ids: BTreeSet<_> = previous.iter().map(|binding| &binding.lock).collect();
    for id in ids {
        super::transfer_stream::check(cancel)?;
        if id != lock.id() {
            guards.push(PairLock::acquire(id)?);
        }
    }
    let reversed_previous = previous.iter().map(reverse).collect();
    import(
        Aliases {
            format: 1,
            current,
            previous,
            complete: stored.as_ref().is_some_and(|saved| saved.complete),
        },
        cancel,
    )?;
    // Ordered state remains ordered. Both orientations share the held
    // physical-pair locks, and an interrupted reverse job remains protected.
    import(
        Aliases {
            format: 1,
            current: reversed,
            previous: reversed_previous,
            complete: reversed_stored.as_ref().is_some_and(|saved| saved.complete),
        },
        cancel,
    )?;
    Ok(guards)
}

fn reverse(binding: &Binding) -> Binding {
    Binding::new(
        [binding.identities[1].clone(), binding.identities[0].clone()],
        [binding.roots[1].clone(), binding.roots[0].clone()],
    )
}
fn import(mut aliases: Aliases, cancel: &AtomicBool) -> io::Result<()> {
    if !aliases.complete {
        // A durable pending record prevents opening a partly copied basis.
        state::save(&aliases)?;
        let mut source = None;
        for old in &aliases.previous {
            let dir = super::replica_state::pair_dir(&old.pair);
            if has_state(&dir, None)? || has_legacy_state(&old.pair)? {
                if source.is_some() {
                    return Err(invalid(
                        "multiple historical pair states need explicit reconciliation",
                    ));
                }
                source = Some(old);
            }
        }
        if let Some(old) = source {
            // Legacy journals, spellings and pending merge inputs live beside
            // the global baseline, even when that baseline is still absent.
            let legacy = super::persistence::baseline_path(&old.pair);
            let parent = legacy
                .parent()
                .ok_or_else(|| invalid("legacy state has no parent"))?;
            let from = format!("baseline_{}.", old.pair);
            let to = format!("baseline_{}.", aliases.current.pair);
            copy_state_dir(
                parent,
                parent,
                old,
                &aliases.current,
                cancel,
                Some((&from, &to)),
            )?;
            copy_state_dir(
                &super::replica_state::pair_dir(&old.pair),
                &super::replica_state::pair_dir(&aliases.current.pair),
                old,
                &aliases.current,
                cancel,
                None,
            )?;
        }
        aliases.complete = true;
    }
    // Backlinks permit immutable old version manifests to retain their pair,
    // backend and lock fields without rewriting or moving any backup data.
    state::save_backlinks(&aliases)?;
    state::save(&aliases)?;
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn has_legacy_state(pair: &str) -> io::Result<bool> {
    let path = super::persistence::baseline_path(pair);
    let parent = path
        .parent()
        .ok_or_else(|| invalid("legacy state has no parent"))?;
    has_state(parent, Some(&format!("baseline_{pair}.")))
}
fn known(name: &str) -> bool {
    name.ends_with(".sebl")
        || name.ends_with(".journal")
        || name.ends_with(".replicas.json")
        || name.ends_with(".dirs.json")
        || name.ends_with(".spellings.json")
        || name.ends_with(".index-dirty")
        || (name.contains(".merge-") && name.ends_with(".json"))
        || (name.contains(".replace-") && name.ends_with(".json"))
}
fn has_state(dir: &Path, prefix: Option<&str>) -> io::Result<bool> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    crate::support_dirs::ensure_private_dir(dir)?;
    let limit = super::SyncLimits::for_memory(crate::transfer::physical_memory()).state_entries;
    for (count, entry) in entries.enumerate() {
        if count as u64 >= limit {
            return Err(invalid("identity state scan exceeds entry budget"));
        }
        let entry = entry?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| prefix.is_none_or(|prefix| name.starts_with(prefix)) && known(name))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn copy_state_dir(
    source: &Path,
    destination: &Path,
    old: &Binding,
    current: &Binding,
    cancel: &AtomicBool,
    rename: Option<(&str, &str)>,
) -> io::Result<()> {
    let entries = match std::fs::read_dir(source) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    crate::support_dirs::ensure_private_dir(source)?;
    crate::support_dirs::ensure_private_dir(destination)?;
    let limits = super::SyncLimits::for_memory(crate::transfer::physical_memory());
    let mut count = 0u64;
    for entry in entries {
        super::transfer_stream::check(cancel)?;
        let entry = entry?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| invalid("identity state filename is not Unicode"))?;
        count = count.saturating_add(1);
        if count > limits.state_entries {
            return Err(invalid("identity migration exceeds entry budget"));
        }
        let target_name = match rename {
            Some((from, to)) => {
                let Some(suffix) = name.strip_prefix(from) else {
                    continue;
                };
                format!("{to}{suffix}")
            }
            None => name.to_string(),
        };
        if known(name) {
            copy_file(
                &entry.path(),
                &destination.join(&target_name),
                old,
                current,
                cancel,
            )?;
        } else if name.contains(".merge-") && name.ends_with(".inputs") {
            crate::support_dirs::ensure_private_dir(&entry.path())?;
            for child in std::fs::read_dir(entry.path())? {
                let child = child?;
                if !matches!(
                    child.file_name().to_str(),
                    Some("original-a" | "original-b" | "merged" | "inputs.json")
                ) {
                    return Err(invalid("unexpected merge input file"));
                }
                copy_file(
                    &child.path(),
                    &destination.join(&target_name).join(child.file_name()),
                    old,
                    current,
                    cancel,
                )?;
            }
        }
    }
    Ok(())
}

fn copy_file(
    source: &Path,
    destination: &Path,
    old: &Binding,
    current: &Binding,
    cancel: &AtomicBool,
) -> io::Result<()> {
    super::transfer_stream::check(cancel)?;
    let file = crate::support_dirs::open_private_file(source)?;
    let limits = super::SyncLimits::for_memory(crate::transfer::physical_memory());
    let max = limits
        .state_file_bytes()
        .saturating_mul(4)
        .max(limits.walk_text_bytes);
    if !file.metadata()?.is_file() || file.metadata()?.len() > max {
        return Err(invalid("identity input exceeds its budget"));
    }
    let mut bytes = Vec::new();
    file.take(max.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(invalid("identity input exceeds its budget"));
    }
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if (name.contains(".merge-") || name.contains(".replace-") || name == "inputs.json")
        && name.ends_with(".json")
    {
        let mut value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if value["pair"] != old.pair || value["lock"] != old.lock {
            return Err(invalid("recovery input has another pair binding"));
        }
        value["pair"] = current.pair.clone().into();
        value["lock"] = current.lock.clone().into();
        if name.contains(".replace-") {
            let side = match value["side"].as_str() {
                Some("a") => 0,
                Some("b") => 1,
                _ => return Err(invalid("replacement input has an invalid side")),
            };
            if value["backend"] != old.identities[side] || value["root"] != old.roots[side] {
                return Err(invalid("replacement input has another endpoint"));
            }
            if value["replica"] == format!("location:{}:{}", old.identities[side], old.roots[side])
            {
                value["replica"] = format!(
                    "location:{}:{}",
                    current.identities[side], current.roots[side]
                )
                .into();
            }
            value["backend"] = current.identities[side].clone().into();
        }
        bytes = serde_json::to_vec_pretty(&value).map_err(io::Error::other)?;
    }
    match crate::support_dirs::open_private_file(destination) {
        Ok(file) => {
            if file.metadata()?.len() != bytes.len() as u64 {
                return Err(invalid(
                    "existing current state differs from historical state",
                ));
            }
            let mut existing = Vec::new();
            file.take(max.saturating_add(1))
                .read_to_end(&mut existing)?;
            if existing != bytes {
                return Err(invalid(
                    "existing current state differs from historical state",
                ));
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            crate::support_dirs::write_private_atomic(destination, &bytes)
        }
        Err(error) => Err(error),
    }
}
