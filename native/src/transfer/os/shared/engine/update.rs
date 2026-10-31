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
//! `batch-mtime-v1`, SFTP and SMB always); otherwise it travels alone, so its
//! stage gets the source's time before it is published (`publish_time.rs`). A
//! copy with the time of the copy would read as newer than its own source. A
//! replacement always travels alone (packets only publish new names).
//!
//! The target is looked at once per file: a local target with one `stat`, a
//! remote target with one tolerant listing per folder (`update_listing.rs`,
//! kept while the job runs, within a byte budget), and not at all in a folder
//! this job created. The decision to replace remembers what it saw
//! (`Replacing`: size, time, provider id); the publication compares the
//! target with it once more right before it replaces (`publish.rs`,
//! `download.rs`, the local copy), so a target saved meanwhile keeps both.
//! A time one side does not know decides nothing: both are kept. The FAT
//! hour needs both files hashed where they are stored (`hour_proof`).
//!
//! Before a folder's files are looked at one by one, its key can settle them
//! (`update_keys.rs`): a source folder with the key of the same-named folder
//! at a remote target is equal for the rule, so neither the target folder is
//! listed nor any of its files compared; the files count as equal and the
//! folders are not even asked for. Only the folders whose keys differ take
//! the way above. A source file with an unknown time (0) never takes the key
//! way: the rule keeps both for it.
//!
//! The key walk also counts what each folder's subtree holds, so a folder
//! with equal keys is not listed at the source either: discovery asks
//! `settled_folder` before it lists a folder, and a settled one is one
//! `WalkEvent::Equal` whose files and bytes are counted as found, equal and
//! skipped at once (`Stats::equal_folder`). A subtree with an omission or a
//! file of unknown time is listed as before.
use super::super::engine_names::{base_name, parent_path, parent_rel};
use super::super::flow::Flow;
use super::super::walk_listers::native;
use super::job_store::with_backend;
use super::ops::OpError;
use super::queue::FileWork;
use super::roots::RootPlan;
use super::stats::Decision;
use super::view::{JobView, Side};
use super::Engine;
use crate::transfer::time_window::{same_content_where_stored, Stored};
use crate::transfer::update_rule::{classify_local_time, window_ms, Case, Replacing, Stamp};
use crate::transfer::{FolderConflict, Layout, RecoveryMode};
use crate::types::Conflict;
use crate::vfs::{KeyTally, LocalBackend, MtimePrecision};
use std::io;
use std::sync::{Arc, Mutex, OnceLock};

#[path = "update_listing.rs"]
mod listings;
use listings::{listing, Listings};
#[path = "update_keys.rs"]
mod keys;
pub(super) use keys::{compare, Keys, Place};

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
pub(super) enum Found {
    Absent,
    /// A folder, link, special file or several entries of that name.
    Other,
    File(Stamp, Option<String>),
}

/// A planned root whose folder decision is "Zusammenführen".
struct MergeRoot {
    /// The planned root with a trailing `/`: the files below it.
    prefix: String,
    /// The planned root (its path relative to the target folder).
    planned: String,
    /// The source folder.
    source: String,
    /// The files below it follow the rule because the folder decision says
    /// so (`FolderConflictPolicy::merge_update`); else only when the job's
    /// policy is `Update`.
    by_decision: bool,
    /// The folders whose keys are equal on both sides, found once by the
    /// first file or folder below the root that asks (`update_keys.rs`).
    equal: OnceLock<Option<Keys>>,
}

/// Per-job state of the rule.
pub(super) struct State {
    /// Planned roots whose folder decision is "Zusammenführen".
    merge_roots: Vec<MergeRoot>,
    /// The files lie below their source folders (`Layout::Tree`): only then
    /// a folder's key says anything about the files below its planned root.
    tree_layout: bool,
    window: OnceLock<u64>,
    local_time: OnceLock<bool>,
    listings: Mutex<Listings>,
}

impl State {
    pub(super) fn new(view: &JobView<'_>, plan: &RootPlan) -> Self {
        let merge_roots = plan
            .roots
            .iter()
            .filter(|root| view.folders.for_root(&root.source) == FolderConflict::Merge)
            .map(|root| MergeRoot {
                prefix: format!("{}/", root.planned.trim_end_matches('/')),
                planned: root.planned.clone(),
                source: root.source.clone(),
                by_decision: view.folders.merge_update,
                equal: OnceLock::new(),
            })
            .collect();
        Self {
            merge_roots,
            tree_layout: view.layout == Layout::Tree,
            window: OnceLock::new(),
            local_time: OnceLock::new(),
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
                .merge_roots
                .iter()
                .any(|root| root.by_decision && file.rel.starts_with(root.prefix.as_str()))
    }

    /// The merge root the rule decides for at `rel` (a file or a folder
    /// below the root, or the root itself) and `rel`'s folder below it.
    fn merge_root_of<'r>(&self, rel: &'r str, is_dir: bool) -> Option<(&MergeRoot, &'r str)> {
        if !self.update.tree_layout {
            return None;
        }
        let root = self.update.merge_roots.iter().find(|root| {
            rel.starts_with(root.prefix.as_str())
                || (is_dir && rel == root.planned.trim_end_matches('/'))
        })?;
        if self.view.conflict != Conflict::Update && !root.by_decision {
            return None;
        }
        let below = rel.get(root.prefix.len()..).unwrap_or("");
        Some((root, if is_dir { below } else { parent_of(below) }))
    }

    /// Whether the key of the folder `dir` of `root` is equal at both sides:
    /// everything in it is equal and needs no look at the target.
    fn keys_cover(&self, root: &MergeRoot, dir: &str) -> bool {
        self.root_keys(root).is_some_and(|keys| keys.covers(dir))
    }

    /// What the folder `rel` (a planned path, in `depth` below its walk root)
    /// holds when its key is equal at both sides and its subtree needs no
    /// look of its own: the source walk neither lists nor sends it, and the
    /// files in it count as equal (`WalkEvent::Equal`). `None` for a folder
    /// that takes the way file by file: not equal, an omission or a file of
    /// unknown time below it, a filtered or flattened job, a resumed one.
    pub(super) fn settled_folder(&self, rel: &str, depth: usize) -> Option<KeyTally> {
        // A resumed job keeps its receipts file by file; a copy inside one
        // namespace excludes its own new destination from the walk, which a
        // tally of the whole subtree would count.
        if self.resume
            || self.view.recovery == RecoveryMode::Resume
            || !self.keeps_folders()
            || self.view.source.same_namespace(self.view.target)
        {
            return None;
        }
        let (root, dir) = self.merge_root_of(rel, true)?;
        self.root_keys(root)?.settled(dir, depth)
    }

    /// The folders of `root` whose keys are equal, found once.
    fn root_keys<'r>(&self, root: &'r MergeRoot) -> Option<&'r Keys> {
        root.equal
            .get_or_init(|| {
                let source = Place {
                    side: self.view.source,
                    root: &root.source,
                    flow: (!self.view.source.is_local()).then_some(&self.source_flow),
                };
                let target_dir = self.folders.path_of(&root.planned);
                let target = Place {
                    side: self.view.target,
                    root: &target_dir,
                    flow: Some(self.target_flow()),
                };
                compare(source, target, self.stop_flag())
            })
            .as_ref()
    }

    /// Whether `file` lies in a folder with equal keys at both sides. A file
    /// with an unknown time is never equal by key: the rule keeps both.
    fn equal_by_key(&self, file: &FileWork) -> bool {
        file.mtime_ms != 0
            && self
                .merge_root_of(&file.rel, false)
                .is_some_and(|(root, dir)| self.keys_cover(root, dir))
    }

    /// The tolerance for comparing times (`window_of`), found once per job.
    fn update_window(&self) -> u64 {
        *self.update.window.get_or_init(|| window_of(&self.view))
    }

    /// Whether a side of the job stores local time (FAT): an hour apart at
    /// equal size is equal (`time_window.rs`), found once per job.
    fn update_local_time(&self) -> bool {
        *self
            .update
            .local_time
            .get_or_init(|| local_time_of(&self.view))
    }

    /// The flow a listing of the remote target takes its permit from.
    pub(super) fn target_flow(&self) -> &Arc<Flow> {
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
    let (source, target) = precisions_of(view);
    window_ms(source, target)
}

/// Whether either side of `view` stores local time (`fs_profile`: FAT), so a
/// clock change can shift its times by exactly one hour. Also used by the
/// folder preview.
pub(super) fn local_time_of(view: &JobView<'_>) -> bool {
    let (source, target) = precisions_of(view);
    source.local_time_shifts() || target.local_time_shifts()
}

/// The time resolutions of the source and the target of `view`.
fn precisions_of(view: &JobView<'_>) -> (MtimePrecision, MtimePrecision) {
    let local = LocalBackend::new("/");
    let precision = |side: Side<'_>, root: &str| match side {
        Side::Local => crate::vfs::mtime_precision(&local, root),
        Side::Remote(backend) => crate::vfs::mtime_precision(backend, root),
    };
    (
        precision(view.source, &view.gate_root()),
        precision(view.target, view.target_dir),
    )
}

/// Decides the rule for `file` when it applies and was not decided yet. An
/// `Err` is a failed look at the target: the file stays undecided and goes
/// alone, where the ordinary path surfaces and retries the failure.
pub(super) fn settle(engine: &Engine<'_>, file: &mut FileWork) -> Result<Settled, OpError> {
    // A stored job that wrote a receipt for this file decides it from the
    // receipt first (`file_recovery::resume`); the rule decides it only when
    // the receipt left it open (`settle_recovered`).
    if engine.view.recovery == RecoveryMode::Resume
        && super::file_recovery::has_receipt(engine, file)
    {
        return Ok(Settled::Go);
    }
    settle_recovered(engine, file)
}

/// `settle` for a file whose receipt (if any) gave no result
/// (`file_recovery::resume` answered `None`): the rule decides it instead of
/// the job's policy keeping both.
pub(super) fn settle_recovered(
    engine: &Engine<'_>,
    file: &mut FileWork,
) -> Result<Settled, OpError> {
    if file.conflict.is_some() || engine.resume || !engine.follows_update(file) {
        return Ok(Settled::Go);
    }
    if engine.equal_by_key(file) {
        return Ok(skip(engine, file, Decision::Equal));
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
            let window = engine.update_window();
            let target_path = engine.folders.path_of(&file.rel);
            let case =
                classify_local_time(source, target, window, engine.update_local_time(), || {
                    hour_proof(engine, &file.source, &target_path)
                });
            if case == Case::SourceNewer {
                file.replacing = Some(Replacing {
                    id,
                    stamp: target,
                    window_ms: window,
                });
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

/// The proof the FAT hour needs: both files hashed where they are stored
/// (`time_window::same_content_where_stored`); `None` where a side cannot.
fn hour_proof(engine: &Engine<'_>, source: &str, target: &str) -> Option<bool> {
    with_backend(engine.view.source, |from| {
        with_backend(engine.view.target, |to| {
            let (from, to) = (Stored::new(from, source), Stored::new(to, target));
            same_content_where_stored(from, to, engine.stop_flag())
        })
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

/// The new folders of `candidates` that follow the rule, created in one
/// request where the target creates folders in bulk (`ensure_many`) before
/// each candidate's `settle` asks for its own parent one by one.
pub(super) fn ensure_parents(engine: &Engine<'_>, candidates: &[FileWork]) {
    let mut parents: Vec<&str> = candidates
        .iter()
        .filter(|file| file.conflict.is_none() && !engine.resume && engine.follows_update(file))
        .filter(|file| !engine.equal_by_key(file))
        .filter_map(|file| parent_rel(&file.rel))
        .collect();
    parents.sort_unstable();
    parents.dedup();
    if !parents.is_empty() {
        engine.folders.ensure_many(&parents, engine.stop_flag());
    }
}

/// Whether the folder `rel` (a planned path, as `Work::Dir` names it) lies
/// in a folder with equal keys at both sides: it exists at the target with
/// everything in it, so it needs neither creating nor a look.
pub(super) fn folder_equal(engine: &Engine<'_>, rel: &str) -> bool {
    !engine.resume
        && engine
            .merge_root_of(rel, true)
            .is_some_and(|(root, dir)| engine.keys_cover(root, dir))
}

fn parent_of(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(parent, _)| parent)
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

#[cfg(test)]
#[path = "abgleich_task_keys_tests.rs"]
mod keys_tests;
#[cfg(test)]
#[path = "abgleich_task_replace_tests.rs"]
mod replace_tests;
#[cfg(test)]
#[path = "abgleich_task_source_keys_tests.rs"]
mod source_keys_tests;
#[cfg(test)]
#[path = "update_tests.rs"]
mod tests;
