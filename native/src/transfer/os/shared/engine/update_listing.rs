//! The target listings of the file rule (`update.rs`, whose submodule this is): one tolerant listing
//! per remote target folder, read once while the job runs and kept within a
//! byte budget.
//!
//! The listing is `vfs::list_dir_tolerant`: a name the host cannot list (not
//! readable, not representable) does not fail the folder. Such a name counts
//! as present and unknown (`Found::Other`, both are kept), so the other files
//! of the folder run as usual.
use super::super::super::flow::classify_error;
use super::super::super::flow_control::OpOutcome;
use super::super::Engine;
use super::Found;
use crate::transfer::update_rule::Stamp;
use crate::vfs::{Backend, VfsMeta};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::sync::{Arc, Mutex, OnceLock};

/// Names of remote folders kept at once: the same 16 MiB of path text the
/// work queue holds, so a job that merges a huge tree stays bounded.
const LISTING_BYTES: usize = 16 * 1024 * 1024;
/// Bookkeeping per listed entry beyond its name.
const ENTRY_OVERHEAD: usize = 64;

pub(super) struct Listing {
    case_sensitive: bool,
    entries: HashMap<String, Vec<VfsMeta>>,
    /// Names the host listed as omissions (folded like `entries`): present,
    /// but nothing is known about them.
    omitted: HashSet<String>,
}

type Failure = (io::ErrorKind, String);
type Slot = Arc<OnceLock<Result<Arc<Listing>, Failure>>>;

#[derive(Default)]
pub(super) struct Listings {
    slots: HashMap<String, Slot>,
    order: VecDeque<(String, usize)>,
    bytes: usize,
}

impl Listing {
    pub(super) fn find(&self, name: &str) -> Found {
        let key = fold(name, self.case_sensitive);
        if self.omitted.contains(&key) {
            return Found::Other;
        }
        match self.entries.get(&key).map(Vec::as_slice) {
            None | Some([]) => Found::Absent,
            Some([entry]) if !entry.is_dir && !entry.is_symlink && !entry.special => Found::File(
                Stamp {
                    size: entry.size,
                    mtime_ms: entry.mtime_ms,
                },
                entry.id.clone(),
            ),
            Some(_) => Found::Other,
        }
    }
}

fn fold(name: &str, case_sensitive: bool) -> String {
    if case_sensitive {
        name.to_string()
    } else {
        name.to_lowercase()
    }
}

/// The listing of the remote folder `dir`, read once while the job runs.
pub(super) fn listing(
    engine: &Engine<'_>,
    target: &dyn Backend,
    dir: &str,
) -> io::Result<Arc<Listing>> {
    let slot = {
        let mut listings = super::super::lock(&engine.update.listings);
        listings
            .slots
            .entry(dir.to_string())
            .or_insert_with(|| Arc::new(OnceLock::new()))
            .clone()
    };
    let result = slot.get_or_init(|| {
        let read = read_listing(engine, target, dir);
        if let Ok(listing) = &read {
            keep(engine, dir, listing);
        }
        read
    });
    match result {
        Ok(listing) => Ok(listing.clone()),
        Err((kind, message)) => {
            // A failed listing is not kept: the next file asks again.
            let mut listings = super::super::lock(&engine.update.listings);
            if listings
                .slots
                .get(dir)
                .is_some_and(|current| Arc::ptr_eq(current, &slot))
            {
                listings.slots.remove(dir);
            }
            Err(io::Error::new(*kind, message.clone()))
        }
    }
}

fn read_listing(
    engine: &Engine<'_>,
    target: &dyn Backend,
    dir: &str,
) -> Result<Arc<Listing>, Failure> {
    let Some(permit) = engine.target_flow().acquire_meta(engine.stop_flag()) else {
        return Err((
            io::ErrorKind::Interrupted,
            super::super::super::cancel::CANCELED_ERROR.to_string(),
        ));
    };
    let listed = crate::vfs::list_dir_tolerant(target, dir);
    permit.finish(match &listed {
        Ok(_) => OpOutcome::Done,
        Err(error) => classify_error(error),
    });
    let listed = match listed {
        Ok(listed) => listed,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Default::default(),
        Err(error) => {
            return Err((error.kind(), format!("Zielordner „{dir}“ lesen: {error}")));
        }
    };
    let case_sensitive = target.case_sensitive_paths(dir);
    let omitted = listed
        .omitted
        .iter()
        .map(|omission| fold(&omission.rel, case_sensitive))
        .collect();
    let mut by_name: HashMap<String, Vec<VfsMeta>> = HashMap::with_capacity(listed.entries.len());
    for entry in listed.entries {
        by_name
            .entry(fold(&entry.name, case_sensitive))
            .or_default()
            .push(entry);
    }
    Ok(Arc::new(Listing {
        case_sensitive,
        entries: by_name,
        omitted,
    }))
}

/// Accounts a new listing and drops the oldest ones beyond the budget.
fn keep(engine: &Engine<'_>, dir: &str, listing: &Listing) {
    let bytes: usize = listing
        .entries
        .keys()
        .chain(listing.omitted.iter())
        .map(|name| name.len() + ENTRY_OVERHEAD)
        .sum();
    let mut listings = super::super::lock(&engine.update.listings);
    listings.order.push_back((dir.to_string(), bytes));
    listings.bytes += bytes;
    while listings.bytes > LISTING_BYTES && listings.order.len() > 1 {
        let Some((oldest, size)) = listings.order.pop_front() else {
            break;
        };
        listings.slots.remove(&oldest);
        listings.bytes = listings.bytes.saturating_sub(size);
    }
}

/// Test access to the folding rule (an omitted name is looked up folded).
#[cfg(test)]
pub(super) fn listing_of(case_sensitive: bool, entries: Vec<VfsMeta>, omitted: &[&str]) -> Listing {
    let mut by_name: HashMap<String, Vec<VfsMeta>> = HashMap::new();
    for entry in entries {
        by_name
            .entry(fold(&entry.name, case_sensitive))
            .or_default()
            .push(entry);
    }
    Listing {
        case_sensitive,
        entries: by_name,
        omitted: omitted
            .iter()
            .map(|name| fold(name, case_sensitive))
            .collect(),
    }
}
