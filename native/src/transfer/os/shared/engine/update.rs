//! The file rule "neuere ersetzen ältere" (`Conflict::Update`) on the engine.
//!
//! The rule is decided once per file, before any copy primitive runs, and the
//! decision is stored in the file (`FileWork::conflict`): a missing target is
//! inserted, equal files and newer targets are skipped, a newer source
//! replaces its older target and a same-time file of another size is kept
//! beside its counterpart. Everything below the engine sees only the
//! concrete policies `Rename` (insert, keep both) and `Overwrite` (replace),
//! so every copy path (local, local packets, download, upload, remote to
//! remote, packets) keeps the protection of the policy it already has.
//!
//! Which files follow the rule: all files of a job whose policy is `Update`
//! and, for jobs with `FolderConflictPolicy::merge_update`, the files below a
//! selected folder whose decision is "Zusammenführen". A file selected on its
//! own keeps the job's policy there.
//!
//! A new file of the rule that goes to a remote target travels in a packet
//! only where the target's packets carry each file's time
//! (`vfs::batch_put_times`: Share `batch_put_mtime_v1`, agent
//! `batch-mtime-v1`); otherwise it travels alone, so its stage gets the
//! source's time before it is published (`publish_time.rs`). A copy with the
//! time of the copy would read as newer than its own source. A replacement
//! always travels alone (packets only publish new names).
//!
//! The target is looked at once per file: a local target with one `stat`, a
//! remote target with one listing per folder (kept while the job runs, within
//! a byte budget), and not at all in a folder this job created.
use super::super::engine_names::{base_name, parent_path, parent_rel};
use super::super::flow::{classify_error, Flow};
use super::super::flow_control::OpOutcome;
use super::super::walk_listers::native;
use super::ops::OpError;
use super::queue::FileWork;
use super::roots::RootPlan;
use super::stats::Decision;
use super::view::{JobView, Side};
use super::Engine;
use crate::transfer::update_rule::{classify, window_ms, Case, Stamp};
use crate::transfer::{FolderConflict, RecoveryMode};
use crate::types::Conflict;
use crate::vfs::{Backend, LocalBackend, VfsMeta};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex, OnceLock};

/// Names of remote folders kept at once: the same 16 MiB of path text the
/// work queue holds, so a job that merges a huge tree stays bounded.
const LISTING_BYTES: usize = 16 * 1024 * 1024;
/// Bookkeeping per listed entry beyond its name.
const ENTRY_OVERHEAD: usize = 64;

/// What the rule did with one file.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Settled {
    /// The file goes on with the policy stored in it.
    Go,
    /// Equal or older than its target: counted, nothing to transfer; the
    /// caller finishes the file as skipped.
    Skipped,
}

/// What the target holds under a file's name.
enum Found {
    Absent,
    /// A folder, link, special file or several entries of that name.
    Other,
    File(Stamp, Option<String>),
}

struct Listing {
    case_sensitive: bool,
    entries: HashMap<String, Vec<VfsMeta>>,
}

type Failure = (io::ErrorKind, String);
type Slot = Arc<OnceLock<Result<Arc<Listing>, Failure>>>;

#[derive(Default)]
struct Listings {
    slots: HashMap<String, Slot>,
    order: VecDeque<(String, usize)>,
    bytes: usize,
}

/// Per-job state of the rule.
pub(super) struct State {
    /// Planned roots (with a trailing `/`) whose folder decision is
    /// "Zusammenführen": the files below them follow the rule when the job
    /// asks for it.
    merge_prefixes: Vec<String>,
    window: OnceLock<u64>,
    listings: Mutex<Listings>,
}

impl State {
    pub(super) fn new(view: &JobView<'_>, plan: &RootPlan) -> Self {
        let merge_prefixes = if view.folders.merge_update {
            plan.roots
                .iter()
                .filter(|root| view.folders.for_root(&root.source) == FolderConflict::Merge)
                .map(|root| format!("{}/", root.planned.trim_end_matches('/')))
                .collect()
        } else {
            Vec::new()
        };
        Self {
            merge_prefixes,
            window: OnceLock::new(),
            listings: Mutex::new(Listings::default()),
        }
    }
}

impl Engine<'_> {
    /// The policy that applies to `file`: the one the rule decided, else the
    /// job's. A job policy of `Update` that was not decided for this file
    /// (a failed look at the target) keeps both, which destroys nothing.
    pub(super) fn conflict_of(&self, file: &FileWork) -> Conflict {
        file.conflict.unwrap_or(match self.view.conflict {
            Conflict::Update => Conflict::Rename,
            other => other,
        })
    }

    /// Whether the file rule decides for `file`.
    fn follows_update(&self, file: &FileWork) -> bool {
        self.view.conflict == Conflict::Update
            || self
                .update
                .merge_prefixes
                .iter()
                .any(|prefix| file.rel.starts_with(prefix.as_str()))
    }

    /// The tolerance for comparing times (`window_of`), found once per job.
    fn update_window(&self) -> u64 {
        *self.update.window.get_or_init(|| window_of(&self.view))
    }

    /// The flow a listing of the remote target takes its permit from.
    fn target_flow(&self) -> &Arc<Flow> {
        match (self.view.source, self.view.target) {
            (Side::Remote(_), Side::Remote(_)) => {
                self.op_flows.1.as_ref().unwrap_or(&self.op_flows.0)
            }
            _ => &self.op_flows.0,
        }
    }
}

/// The tolerance for comparing times in `view`: the coarser resolution of
/// the two sides, two seconds where a side does not know its own. Also the
/// tolerance of the folder preview.
pub(super) fn window_of(view: &JobView<'_>) -> u64 {
    let local = LocalBackend::new("/");
    let precision = |side: Side<'_>, root: &str| match side {
        Side::Local => crate::vfs::mtime_precision(&local, root),
        Side::Remote(backend) => crate::vfs::mtime_precision(backend, root),
    };
    window_ms(
        precision(view.source, &view.gate_root()),
        precision(view.target, view.target_dir),
    )
}

/// Decides the rule for `file` when it applies and was not decided yet. An
/// `Err` is a failed look at the target: the file stays undecided and goes
/// alone, where the ordinary path surfaces and retries the failure.
pub(super) fn settle(engine: &Engine<'_>, file: &mut FileWork) -> Result<Settled, OpError> {
    if file.conflict.is_some() || engine.resume || !engine.follows_update(file) {
        return Ok(Settled::Go);
    }
    // A stored job that wrote a receipt for this file decides it from the
    // receipt first (`file_recovery::resume`).
    if engine.view.recovery == RecoveryMode::Resume
        && super::file_recovery::has_receipt(engine, file)
    {
        return Ok(Settled::Go);
    }
    let found = look(engine, file).map_err(OpError::target)?;
    let case = match found {
        Found::Absent => {
            file.conflict = Some(Conflict::Rename);
            file.alone |= !packets_keep_time(engine);
            return Ok(Settled::Go);
        }
        Found::Other => Case::Conflict,
        Found::File(target, id) => {
            let source = Stamp {
                size: file.size,
                mtime_ms: file.mtime_ms,
            };
            let case = classify(source, target, engine.update_window());
            if case == Case::SourceNewer {
                file.replacing = Some(id);
            }
            case
        }
    };
    Ok(match case {
        Case::Equal => skip(engine, file, Decision::Equal),
        Case::TargetNewer => {
            engine
                .issues
                .note(&file.source, "Ziel ist neuer – behalten, nicht ersetzt");
            skip(engine, file, Decision::TargetNewer)
        }
        Case::SourceNewer => {
            engine.stats.decided(Decision::Replaced);
            file.conflict = Some(Conflict::Overwrite);
            file.alone = true;
            Settled::Go
        }
        Case::Conflict => {
            engine.issues.note(
                &file.source,
                "Konflikt: gleiche Änderungszeit, andere Größe – beide behalten",
            );
            engine.stats.decided(Decision::Both);
            file.conflict = Some(Conflict::Rename);
            file.alone |= !packets_keep_time(engine);
            Settled::Go
        }
    })
}

/// Whether a new file may go in a packet without losing its source's time:
/// a local target (its packets publish stages the engine timed) or a remote
/// one whose packets carry the time.
pub(super) fn packets_keep_time(engine: &Engine<'_>) -> bool {
    match engine.view.target {
        Side::Local => true,
        Side::Remote(target) => crate::vfs::batch_put_times(target),
    }
}

/// A file the rule leaves alone: counted as handled so the progress ends at
/// its full size. The caller finishes it as skipped (`worker::finished`).
fn skip(engine: &Engine<'_>, file: &FileWork, decision: Decision) -> Settled {
    engine.stats.decided(decision);
    engine.stats.credit(file.size);
    Settled::Skipped
}

/// What the target holds under the name of `file`.
fn look(engine: &Engine<'_>, file: &FileWork) -> io::Result<Found> {
    // A folder this job created holds nothing of anyone else.
    if let Some(parent) = parent_rel(&file.rel) {
        match engine.folders.ensure(parent, engine.stop_flag()) {
            Ok(true) => return Ok(Found::Absent),
            Ok(false) => {}
            Err(error) => return Err(io::Error::new(error.kind, error.message)),
        }
    }
    let path = engine.folders.path_of(&file.rel);
    match engine.view.target {
        Side::Local => look_local(&path),
        Side::Remote(target) => {
            let listing = listing(engine, target, &parent_path(&path))?;
            Ok(listing.find(base_name(&path)))
        }
    }
}

fn look_local(path: &str) -> io::Result<Found> {
    let native = native(path);
    match crate::local_access::symlink_metadata(&native) {
        Ok(metadata) => {
            if !metadata.is_file() || crate::local_access::metadata_is_link_like(&native, &metadata)
            {
                return Ok(Found::Other);
            }
            let mtime_ms = metadata
                .modified()
                .map(crate::local_access::system_time_ms)
                .unwrap_or(0);
            let stamp = Stamp {
                size: metadata.len(),
                mtime_ms,
            };
            Ok(Found::File(stamp, None))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Found::Absent),
        Err(error) => Err(error),
    }
}

impl Listing {
    fn find(&self, name: &str) -> Found {
        let key = fold(name, self.case_sensitive);
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
fn listing(engine: &Engine<'_>, target: &dyn Backend, dir: &str) -> io::Result<Arc<Listing>> {
    let slot = {
        let mut listings = super::lock(&engine.update.listings);
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
            let mut listings = super::lock(&engine.update.listings);
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
            super::super::cancel::CANCELED_ERROR.to_string(),
        ));
    };
    let listed = target.list_dir(dir);
    permit.finish(match &listed {
        Ok(_) => OpOutcome::Done,
        Err(error) => classify_error(error),
    });
    let entries = match listed {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            return Err((error.kind(), format!("Zielordner „{dir}“ lesen: {error}")));
        }
    };
    let case_sensitive = target.case_sensitive_paths(dir);
    let mut by_name: HashMap<String, Vec<VfsMeta>> = HashMap::with_capacity(entries.len());
    for entry in entries {
        by_name
            .entry(fold(&entry.name, case_sensitive))
            .or_default()
            .push(entry);
    }
    Ok(Arc::new(Listing {
        case_sensitive,
        entries: by_name,
    }))
}

/// Accounts a new listing and drops the oldest ones beyond the budget.
fn keep(engine: &Engine<'_>, dir: &str, listing: &Listing) {
    let bytes: usize = listing
        .entries
        .keys()
        .map(|name| name.len() + ENTRY_OVERHEAD)
        .sum();
    let mut listings = super::lock(&engine.update.listings);
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

#[cfg(test)]
#[path = "update_tests.rs"]
mod tests;
