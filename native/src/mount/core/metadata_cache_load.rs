use crate::vfs::VfsMeta;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, Weak};

pub(in crate::mount) enum MetadataLookup {
    Found(VfsMeta),
    KnownMissing,
    Uncached,
}

pub(in crate::mount) struct LoadSlot {
    gate: Mutex<()>,
    revision: AtomicU64,
}

impl LoadSlot {
    pub(super) fn new() -> Self {
        Self {
            gate: Mutex::new(()),
            revision: AtomicU64::new(0),
        }
    }

    pub(in crate::mount) fn lock(&self) -> io::Result<MutexGuard<'_, ()>> {
        self.gate
            .lock()
            .map_err(|_| io::Error::other("metadata load slot is unavailable"))
    }

    pub(in crate::mount) fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }

    pub(super) fn invalidate(&self) {
        self.revision.fetch_add(1, Ordering::AcqRel);
    }
}

pub(super) fn retain_active_loads(loads: &mut HashMap<String, Weak<LoadSlot>>) {
    loads.retain(|_, slot| slot.strong_count() > 0);
}

pub(super) fn invalidate_slot(loads: &mut HashMap<String, Weak<LoadSlot>>, key: &str) {
    retain_active_loads(loads);
    if let Some(slot) = loads.get(key).and_then(Weak::upgrade) {
        slot.invalidate();
    }
}

pub(super) fn invalidate_descendants(loads: &mut HashMap<String, Weak<LoadSlot>>, parent: &str) {
    retain_active_loads(loads);
    let prefix = format!("{}/", parent.trim_end_matches('/'));
    for (candidate, slot) in loads.iter() {
        if candidate != parent && candidate.starts_with(&prefix) {
            if let Some(slot) = slot.upgrade() {
                slot.invalidate();
            }
        }
    }
}

pub(super) fn invalidate_paths(
    loads: &mut HashMap<String, Weak<LoadSlot>>,
    key: &str,
    prefix: &str,
    recursive: bool,
    parent: Option<&str>,
) {
    retain_active_loads(loads);
    for (candidate, slot) in loads.iter() {
        let affected = candidate == key
            || (recursive && candidate.starts_with(prefix))
            || parent == Some(candidate.as_str());
        if affected {
            if let Some(slot) = slot.upgrade() {
                slot.invalidate();
            }
        }
    }
}
