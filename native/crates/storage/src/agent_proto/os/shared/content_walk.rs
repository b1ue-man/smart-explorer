//! The parallel walk of a content search: the work-stealing tree walk of the
//! hash walks (`ordered_walk`, like ripgrep's `WalkParallel`: every thread
//! reads the folders and searches the files it finds; an idle thread takes
//! over folders and file groups of a busy one). Links and special files are
//! never entered or read. The tree is a trait, so the same walk searches a
//! local folder (the app, the SSH agent) and any backend (`vfs::content_search`).
use std::io::{self, Read};
use std::sync::atomic::AtomicBool;

use super::content_filter::PathFilter;
use super::content_match::ContentMatcher;
use super::content_query::{ContentQuery, ContentSummary, SkipReason};
use super::content_run::{ContentEmit, ContentRun};
use super::ordered_walk::{self, Local, Output};

/// Files one job searches; a folder with more becomes several jobs, so idle
/// threads share a large folder.
const FILES_PER_JOB: usize = 16;

/// What an entry of a listed folder is to the search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeKind {
    Folder,
    File,
    /// A link, special file or anything else: never entered or read.
    Other,
    /// An entry that cannot be used (its name, its metadata); the text names why.
    Unreadable(String),
}

/// One entry of a listed folder.
#[derive(Clone, Debug)]
pub struct TreeEntry {
    pub name: String,
    pub kind: TreeKind,
    pub hidden: bool,
    pub size: u64,
    pub mtime_ms: i64,
    /// Backend-unique id (Drive), passed back to `ContentTree::open`.
    pub id: Option<String>,
}

/// A tree a content search walks. Paths are relative to the search root,
/// `/`-separated, "" = the root itself.
pub trait ContentTree: Sync {
    /// The root when it is a file instead of a folder: then only it is searched.
    fn root_file(&self) -> io::Result<Option<TreeEntry>> {
        Ok(None)
    }
    /// The entries of the folder `rel`.
    fn list(&self, rel: &str) -> io::Result<Vec<TreeEntry>>;
    /// The content of the file `rel` (listed as `entry`), without following a link.
    fn open(&self, rel: &str, entry: &TreeEntry) -> io::Result<Box<dyn Read + '_>>;
}

enum Job {
    Folder {
        rel: String,
        level: usize,
    },
    Files {
        folder: String,
        files: Vec<TreeEntry>,
    },
}

/// Nothing is buffered per thread: events go out as each file is searched.
struct Direct;

impl Output for Direct {
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// `rel` of `name` inside the folder `folder`.
pub fn join_rel(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_string()
    } else {
        format!("{folder}/{name}")
    }
}

/// The matcher and filter of `query`, the errors as `InvalidInput` (German).
pub fn prepare(query: &ContentQuery) -> io::Result<(ContentMatcher, PathFilter)> {
    let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidInput, message);
    Ok((
        ContentMatcher::new(query).map_err(invalid)?,
        PathFilter::new(query).map_err(invalid)?,
    ))
}

/// Searches `tree` on `threads` threads, every event into `emit`. `Err` for
/// an invalid query, an unlistable root, a cancel or a receiver that is gone.
pub fn search_tree(
    tree: &dyn ContentTree,
    query: &ContentQuery,
    threads: usize,
    emit: ContentEmit<'_>,
    cancel: &AtomicBool,
) -> io::Result<ContentSummary> {
    let (matcher, filter) = prepare(query)?;
    let run = ContentRun::new(&matcher, query, emit, cancel);
    if let Some(file) = tree.root_file()? {
        let mut open = || tree.open("", &file);
        let searched = run.search_reader(&file.name, file.size, file.mtime_ms, &mut open);
        return run.finish(searched.map(drop));
    }
    let roots = vec![Job::Folder {
        rel: String::new(),
        level: 0,
    }];
    let walked = ordered_walk::run(
        roots,
        threads.max(1),
        cancel,
        || Direct,
        |job, local: &mut Local<Job>, _: &mut Direct| match job {
            Job::Folder { rel, level } => visit_folder(tree, &run, &filter, (&rel, level), local),
            Job::Files { folder, files } => search_files(tree, &run, &folder, &files),
        },
    );
    run.finish(walked)
}

fn visit_folder(
    tree: &dyn ContentTree,
    run: &ContentRun<'_>,
    filter: &PathFilter,
    (rel, level): (&str, usize),
    local: &mut Local<Job>,
) -> io::Result<()> {
    halt_walk(run)?;
    let entries = match tree.list(rel) {
        Ok(entries) => entries,
        // The root itself must be listable; below it, a folder is reported.
        Err(error) if rel.is_empty() => return Err(error),
        Err(error) => {
            run.skip(rel, SkipReason::UnlistableFolder(error.to_string()));
            return Ok(());
        }
    };
    let search_here = filter.search_in_folder(rel);
    let mut files = Vec::new();
    for entry in entries {
        match entry.kind {
            TreeKind::Folder => {
                let child = join_rel(rel, &entry.name);
                if filter.admit_folder(&child, level + 1, &entry.name, entry.hidden) {
                    local.push(Job::Folder {
                        rel: child,
                        level: level + 1,
                    });
                }
            }
            TreeKind::File => {
                if search_here && filter.admit_file(&entry.name) {
                    files.push(entry);
                }
            }
            TreeKind::Other => {}
            TreeKind::Unreadable(reason) => {
                run.skip(&join_rel(rel, &entry.name), SkipReason::Unreadable(reason));
            }
        }
    }
    while !files.is_empty() {
        let rest = files.split_off(files.len().min(FILES_PER_JOB));
        local.push(Job::Files {
            folder: rel.to_string(),
            files: std::mem::replace(&mut files, rest),
        });
    }
    Ok(())
}

fn search_files(
    tree: &dyn ContentTree,
    run: &ContentRun<'_>,
    folder: &str,
    files: &[TreeEntry],
) -> io::Result<()> {
    for file in files {
        halt_walk(run)?;
        let rel = join_rel(folder, &file.name);
        let mut open = || tree.open(&rel, file);
        run.search_reader(&rel, file.size, file.mtime_ms, &mut open)?;
    }
    Ok(())
}

/// Ends the walk once the search stopped (the run then says why).
fn halt_walk(run: &ContentRun<'_>) -> io::Result<()> {
    run.check()?;
    if run.halted() {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "content search reached its hit limit",
        ));
    }
    Ok(())
}
