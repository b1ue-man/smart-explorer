//! Recover exactly journaled bytes with create-only restoration; never replace a new creator.
use super::incremental::SyncEndpoints;
use super::replacement_journal::{self as journal, Intent, Observed};
use super::run_types::StateKey;
use crate::vfs::Backend;
use std::io;
use std::sync::atomic::AtomicBool;

pub(super) fn recover_locked(
    lock: &super::PairLock,
    key: &StateKey,
    endpoints: SyncEndpoints<'_>,
    cross_mounts: bool,
    cancel: &AtomicBool,
) -> io::Result<()> {
    for intent in collect_owned(lock, key, endpoints)? {
        super::transfer_stream::check(cancel)?;
        let backend = if intent.binding.side == "a" {
            endpoints.a
        } else {
            endpoints.b
        };
        super::apply_boundary::guard(
            backend,
            &intent.binding.root,
            &intent.binding.rel,
            cross_mounts,
        )?;
        recover(backend, &intent, cancel)?;
    }
    Ok(())
}

pub(super) fn relatives(
    lock: &super::PairLock,
    key: &StateKey,
    endpoints: SyncEndpoints<'_>,
) -> io::Result<Vec<String>> {
    Ok(collect_owned(lock, key, endpoints)?
        .into_iter()
        .map(|intent| intent.binding.rel)
        .collect())
}

fn collect_owned(
    lock: &super::PairLock,
    key: &StateKey,
    endpoints: SyncEndpoints<'_>,
) -> io::Result<Vec<Intent>> {
    if lock.id() != key.lock_id {
        return Err(io::Error::other(
            "replacement recovery has another pair lock",
        ));
    }
    let dir = super::replica_state::pair_dir(&key.pair_id);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let limits = super::SyncLimits::for_memory(crate::transfer::physical_memory());
    let mut count = 0u64;
    let mut bytes = 0u64;
    let mut intents = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.contains(".replace-") || !name.ends_with(".json") {
            continue;
        }
        let owner = name
            .split_once(".replace-")
            .map(|(owner, _)| owner)
            .ok_or_else(|| io::Error::other("replacement filename has no owner"))?;
        super::replica_state::owner_from_token(owner)?;
        count = count.saturating_add(1);
        if count > limits.state_entries {
            return Err(io::Error::other(
                "replacement recovery exceeds its entry budget",
            ));
        }
        let text = crate::support_dirs::read_private_text(&entry.path(), 256 * 1024)?;
        bytes = bytes.saturating_add(text.len() as u64);
        if bytes > limits.state_text_bytes {
            return Err(io::Error::other(
                "replacement recovery exceeds its text budget",
            ));
        }
        let intent: Intent = serde_json::from_str(&text).map_err(io::Error::other)?;
        if intent.binding.pair != key.pair_id
            || intent.binding.lock != key.lock_id
            || intent.binding.owner != owner
            || intent.path()? != entry.path()
        {
            return Err(io::Error::other(
                "replacement intent belongs to another state",
            ));
        }
        let (backend, root) = match intent.binding.side.as_str() {
            "a" => (endpoints.a, endpoints.root_a),
            "b" => (endpoints.b, endpoints.root_b),
            _ => return Err(io::Error::other("replacement intent has an invalid side")),
        };
        if intent.binding.root != root {
            return Err(io::Error::other(
                "replacement intent belongs to another root",
            ));
        }
        intent.validate(backend)?;
        // A rotated physical replica never inherits the old intent's mutation.
        if intent.binding.replica != super::version_save::replica(backend, root)? {
            continue;
        }
        intents.push(intent);
    }
    Ok(intents)
}

fn recover(backend: &dyn Backend, intent: &Intent, cancel: &AtomicBool) -> io::Result<()> {
    let original = journal::observe(backend, &intent.destination, cancel)?;
    let retained = journal::observe(backend, &intent.retained, cancel)?;
    let stage = journal::observe(backend, &intent.stage, cancel)?;
    if let Some(stage) = stage.as_ref() {
        journal::require_match(Some(stage), &intent.staged)?;
    }
    if let Some(retained) = retained.as_ref() {
        journal::require_match(Some(retained), &intent.original)?;
    }
    match original.as_ref() {
        Some(actual) if journal::matches(actual, &intent.staged) => {
            // A lost publish ACK is resolved by the actual destination's
            // content and namespace confirmation. No baseline is manufactured:
            // the full planner observes the current pair and records it, and
            // recorded merges keep their independent durable recovery inputs.
            return finish_published(backend, intent, cancel);
        }
        Some(actual) if journal::matches(actual, &intent.original) => {
            // No mutation, or a confirmed create-only rollback. Its old
            // baseline remains authoritative; an ordinary retry stages anew.
            super::apply_stage::require_durable(super::apply_stage::namespace(
                backend,
                &intent.destination,
            )?)?;
            if let Some(stage) = stage {
                erase_known(backend, &intent.stage, &stage, cancel)?;
            }
            if let Some(retained) = retained {
                erase_known(backend, &intent.retained, &retained, cancel)?;
            }
        }
        None => {
            let retained = retained
                .as_ref()
                .ok_or_else(|| uncertain(intent, "the original is absent from both known slots"))?;
            // Recheck the full original content immediately before a
            // NoReplace restore; a third-party creator wins without overwrite.
            journal::require_match(
                journal::observe(backend, &intent.retained, cancel)?.as_ref(),
                retained,
            )?;
            super::transfer_stream::check(cancel)?;
            backend.rename_no_replace(&intent.retained, &intent.destination)?;
            journal::require_match(
                journal::observe(backend, &intent.destination, cancel)?.as_ref(),
                &intent.original,
            )?;
            super::apply_stage::require_durable(super::apply_stage::namespace(
                backend,
                &intent.destination,
            )?)?;
            if let Some(stage) = stage {
                erase_known(backend, &intent.stage, &stage, cancel)?;
            }
        }
        Some(_) => {
            return Err(uncertain(
                intent,
                "new foreign destination bytes are preserved",
            ))
        }
    }
    journal::remove(intent)
}

/// Retire only a verified publication, never the rollback or foreign-byte branch.
pub(super) fn finish_published(
    backend: &dyn Backend,
    intent: &Intent,
    cancel: &AtomicBool,
) -> io::Result<()> {
    super::transfer_stream::check(cancel)?;
    intent.validate(backend)?;
    journal::require_match(
        journal::observe(backend, &intent.destination, cancel)?.as_ref(),
        &intent.staged,
    )?;
    let retained = journal::observe(backend, &intent.retained, cancel)?;
    let stage = journal::observe(backend, &intent.stage, cancel)?;
    if let Some(retained) = retained.as_ref() {
        journal::require_match(Some(retained), &intent.original)?;
    }
    if let Some(stage) = stage.as_ref() {
        journal::require_match(Some(stage), &intent.staged)?;
    }
    super::apply_stage::require_durable(super::apply_stage::namespace(
        backend,
        &intent.destination,
    )?)?;
    // Validate both slots before removing either, then recheck each exact
    // observed identity immediately before its non-recursive removal.
    if let Some(retained) = retained {
        erase_known(backend, &intent.retained, &retained, cancel)?;
    }
    if let Some(stage) = stage {
        erase_known(backend, &intent.stage, &stage, cancel)?;
    }
    journal::require_match(
        journal::observe(backend, &intent.destination, cancel)?.as_ref(),
        &intent.staged,
    )?;
    super::transfer_stream::check(cancel)?;
    journal::remove(intent)
}

fn erase_known(
    backend: &dyn Backend,
    path: &str,
    expected: &Observed,
    cancel: &AtomicBool,
) -> io::Result<()> {
    let observed = journal::observe(backend, path, cancel)?;
    journal::require_match(observed.as_ref(), expected)?;
    super::transfer_stream::check(cancel)?;
    backend.remove_file_id(path, expected.id.as_deref())?;
    super::apply_stage::require_durable(super::apply_stage::namespace(backend, path)?)
}
fn uncertain(intent: &Intent, detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::WouldBlock, format!(
        "replacement outcome remains uncertain ({detail}); preserve stage={}, destination={}, retained={}, intent={}",
        intent.stage, intent.destination, intent.retained, intent.path().map_or_else(|_| "unknown".into(), |p| p.display().to_string())))
}
