//! The files of a folder staged for other programs, downloaded side by side.
//!
//! The flow of the connection decides how many run at once (`limit` of its
//! snapshot, and only while it has a free permit); every file runs under a
//! permit of the shared transfer lane, so a transfer started from the
//! transfer window and this staging take turns. The first failure stops the
//! remaining files and is returned as it came.
use super::downloads::download_collected_file;
use super::entries::RemoteFileEntry;
use super::flow::{classify_error, flow_for, Flow};
use super::OpOutcome;
use std::path::{Path, MAIN_SEPARATOR_STR};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Job number of the staging for other programs: between the numbers the
/// engine hands out from 1 and the sync runs from `1 << 48`.
pub(super) const CLIPBOARD_JOB: u64 = 1 << 47;
/// How often the starter looks at the flow while all permits are taken.
const POLL: Duration = Duration::from_millis(20);

/// Points where a test replaces the system.
pub(super) struct Hooks<'a> {
    /// Whether the system gives the worker with this index a thread.
    pub(super) may_spawn: &'a (dyn Fn(usize) -> bool + Sync),
}

pub(super) fn download_files_parallel(
    be: &dyn crate::vfs::Backend,
    files: &[RemoteFileEntry],
    local_dir: &Path,
) -> Result<(), String> {
    download_files_with(
        be,
        files,
        local_dir,
        &Hooks {
            may_spawn: &|_| true,
        },
    )
}

pub(super) fn download_files_with(
    be: &dyn crate::vfs::Backend,
    files: &[RemoteFileEntry],
    local_dir: &Path,
    hooks: &Hooks<'_>,
) -> Result<(), String> {
    match files {
        [] => return Ok(()),
        // One file is one call, as it always was: no thread, no permit.
        [file] => return download_into(be, file, local_dir),
        _ => {}
    }
    let run = Run {
        be,
        files,
        local_dir,
        flow: flow_for(be, &files[0].src),
        next: AtomicUsize::new(0),
        running: AtomicUsize::new(0),
        abort: AtomicBool::new(false),
        first_error: Mutex::new(None),
    };
    std::thread::scope(|scope| {
        let mut started = 0;
        while !run.abort.load(Ordering::Acquire) && run.next.load(Ordering::Acquire) < files.len() {
            let running = run.running.load(Ordering::Acquire);
            let room = running < run.flow.snapshot().limit.max(1);
            // Somebody has to make progress even when others hold every
            // permit; the first worker waits its turn in the flow.
            let may_start = room && (running == 0 || run.flow.has_spare());
            if !may_start {
                std::thread::sleep(POLL);
                continue;
            }
            if !(hooks.may_spawn)(started) {
                break;
            }
            run.running.fetch_add(1, Ordering::AcqRel);
            let spawned = std::thread::Builder::new()
                .name("clipboard-download".into())
                .spawn_scoped(scope, || run.work());
            if spawned.is_err() {
                run.running.fetch_sub(1, Ordering::AcqRel);
                break;
            }
            started += 1;
        }
        // No thread (or no more): the calling thread takes what is left,
        // one file after the other, while the started workers go on.
        if run.next.load(Ordering::Acquire) < files.len() && !run.abort.load(Ordering::Acquire) {
            run.work_inline();
        }
    });
    let mut first = run
        .first_error
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match first.take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct Run<'a> {
    be: &'a dyn crate::vfs::Backend,
    files: &'a [RemoteFileEntry],
    local_dir: &'a Path,
    flow: Arc<Flow>,
    next: AtomicUsize,
    running: AtomicUsize,
    abort: AtomicBool,
    first_error: Mutex<Option<String>>,
}

impl Run<'_> {
    /// A worker thread: takes files until none is left or one failed.
    fn work(&self) {
        let _guard = Ending(self);
        self.work_inline();
    }

    fn work_inline(&self) {
        while !self.abort.load(Ordering::Acquire) {
            let index = self.next.fetch_add(1, Ordering::AcqRel);
            let Some(file) = self.files.get(index) else {
                return;
            };
            let Some(permit) = self.flow.acquire_for(CLIPBOARD_JOB, &self.abort) else {
                return;
            };
            match download_into(self.be, file, self.local_dir) {
                Ok(()) => {
                    permit.progress(file.size);
                    permit.finish(OpOutcome::Done);
                }
                Err(error) => {
                    permit.finish(classify_error(&std::io::Error::other(error.clone())));
                    self.fail(error);
                }
            }
        }
    }

    fn fail(&self, error: String) {
        self.first_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_or_insert(error);
        self.abort.store(true, Ordering::Release);
    }
}

/// Counts a worker out when it ends, also by a panic (which then stops the
/// others as well).
struct Ending<'a, 'b>(&'a Run<'b>);

impl Drop for Ending<'_, '_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.abort.store(true, Ordering::Release);
        }
        self.0.running.fetch_sub(1, Ordering::AcqRel);
    }
}

fn download_into(
    be: &dyn crate::vfs::Backend,
    file: &RemoteFileEntry,
    local_dir: &Path,
) -> Result<(), String> {
    let dest = local_dir.join(file.rel.replace('/', MAIN_SEPARATOR_STR));
    if be.is_local() {
        return copy_local_into(file, &dest);
    }
    download_collected_file(be, file, &dest).map(|_| ())
}

/// A local file (the long-path clipboard of a local selection, M18,
/// 2026-10-28) is copied by the kernel (`copy_file_range`, `CopyFile2`) into
/// a fresh stage beside `dest` instead of through a user-space read/write
/// loop: the copy checks the expected size and that the source did not
/// change, and its medium arbitration is the copy's. The stage is published
/// without replacing anything; like every staging for other programs it is
/// not synced (the session's temp folder is disposable).
fn copy_local_into(file: &RemoteFileEntry, dest: &Path) -> Result<(), String> {
    let source = super::walk_listers::native(&file.src);
    let never = AtomicBool::new(false);
    let mut last = None;
    for attempt in 0..STAGE_ATTEMPTS {
        let stage = stage_beside(dest, attempt);
        match crate::copy::copy_to_new_file(&source, &stage, file.size, &never) {
            Ok(_) => {
                if let Err(error) = crate::vfs::promote_local_copy(&stage, dest) {
                    let _ = std::fs::remove_file(&stage);
                    return Err(format!(
                        "Ziel „{}“ ohne Ersetzen veröffentlichen: {error}",
                        dest.display()
                    ));
                }
                return Ok(());
            }
            // A foreign entry of that exact name: never ours, next name.
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => last = Some(error),
            Err(error) => return Err(format!("{}: {error}", file.src)),
        }
    }
    Err(format!(
        "{}: keine freie Kopierstufe: {}",
        file.src,
        last.map_or_else(String::new, |error| error.to_string())
    ))
}

/// Names tried for one stage.
const STAGE_ATTEMPTS: u32 = 16;

/// An unpredictable sibling name of `dest` for its stage.
fn stage_beside(dest: &Path, attempt: u32) -> std::path::PathBuf {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    let name = dest
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "copy".to_string());
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let tail = format!(".se-clip-{nonce:x}-{attempt}.part");
    parent.join(crate::vfs::fit_stage_name(".", &name, &tail))
}
