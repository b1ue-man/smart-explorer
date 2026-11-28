//! `se search` and the MCP `search` tool: name search below a target,
//! server-side where the backend has it, otherwise a bounded listing walk.
//! The walk reports links and special files whose names match, as such, and
//! never enters a link (loops such as `sub/loop -> ..` and targets outside
//! the searched tree stay out of the result).
//!
//! Beyond names (plan docs/plaene/2026-11-28-ersetzen-masken-shell, part C):
//! a glob with `/` or `**` matches the relative path (`search_glob.rs`),
//! `gitignore` leaves out what the ignore files exclude (`search_ignore.rs`),
//! and `sort` orders the hits by path or by modification time (newest first).
//! A search next to the data then still filters names there (the last path
//! component where it is a plain glob) and the rest is checked here.
use super::entry_kind::EntryKind;
use super::target::Target;
use crate::agent_proto::SearchSpec;
use crate::vfs::{Backend, SearchHit, VfsMeta};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};

#[path = "search_glob.rs"]
mod glob;
#[path = "search_ignore.rs"]
mod ignore_files;

const MAX_SEARCH_NODES: u64 = 1_000_000;
const MAX_SEARCH_TEXT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SEARCH_DEPTH: usize = 512;

pub(super) const SEARCH_CANCELLED: &str = "search cancelled";

/// One search result, relative to the searched root.
pub(super) struct Found {
    pub(super) rel: String,
    pub(super) kind: EntryKind,
    pub(super) size: u64,
    pub(super) mtime_ms: i64,
}

impl Found {
    fn from_server(hit: SearchHit) -> Self {
        // Server-side searches (SSH agent, Share peers) skip links and
        // special files themselves; their protocol knows files and folders.
        Self {
            kind: if hit.is_dir {
                EntryKind::Dir
            } else {
                EntryKind::File
            },
            rel: hit.rel,
            size: hit.size,
            mtime_ms: hit.mtime_ms,
        }
    }
}

#[derive(Default)]
struct SearchBudget {
    nodes: u64,
    text_bytes: u64,
}

impl SearchBudget {
    fn record(&mut self, path: &str, depth: usize) -> Result<(), String> {
        if depth > MAX_SEARCH_DEPTH {
            return Err(format!("search tree exceeds {MAX_SEARCH_DEPTH} levels"));
        }
        self.nodes = self.nodes.saturating_add(1);
        self.text_bytes = self.text_bytes.saturating_add(path.len() as u64);
        if self.nodes > MAX_SEARCH_NODES || self.text_bytes > MAX_SEARCH_TEXT_BYTES {
            return Err("search tree exceeds its bounded collection budget".to_string());
        }
        Ok(())
    }
}

/// The order of the hits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SearchSort {
    /// As the search finds them (streamed).
    #[default]
    Found,
    /// By relative path.
    Name,
    /// By modification time, newest first (then by path).
    Mtime,
}

impl SearchSort {
    pub(super) fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "" | "none" | "found" => Some(Self::Found),
            "name" | "path" => Some(Self::Name),
            "mtime" | "modified" | "time" => Some(Self::Mtime),
            _ => None,
        }
    }

    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Found => "none",
            Self::Name => "name",
            Self::Mtime => "mtime",
        }
    }
}

/// One name search: `se search` and the MCP `search` tool.
#[derive(Clone, Debug, Default)]
pub(super) struct SearchOptions {
    pub(super) query: String,
    pub(super) glob: bool,
    pub(super) max_results: u64,
    pub(super) want_dirs: bool,
    pub(super) sort: SearchSort,
    pub(super) gitignore: bool,
}

pub(crate) fn search(
    target: &Target,
    query: &str,
    glob: bool,
    max_results: u64,
    want_dirs: bool,
) -> Result<(), String> {
    let spec = search_spec(query, glob, max_results, want_dirs);
    let cancel = AtomicBool::new(false);
    search_with(target, &spec, &cancel, &mut |found| {
        println!("{}\t{}\t{}", found.kind.marker(), found.size, found.rel)
    })
}

/// `se search` with every option: the former lines (`marker`, size, path)
/// or, with `json`, one JSON object per hit as the MCP tool streams it.
pub(super) fn search_cli(
    target: &Target,
    options: &SearchOptions,
    json: bool,
) -> Result<(), String> {
    let cancel = AtomicBool::new(false);
    search_found(target, options, &cancel, &mut |found| {
        if json {
            println!("{}", hit_json(&target.path, &found));
        } else {
            println!("{}\t{}\t{}", found.kind.marker(), found.size, found.rel)
        }
    })
}

/// One hit as JSON: `path`, `relative`, `type`, `size`, `mtime_ms`.
pub(super) fn hit_json(root: &str, found: &Found) -> Value {
    json!({
        "path": super::ops::join(root, &found.rel),
        "relative": found.rel,
        "type": found.kind.label(),
        "size": found.size,
        "mtime_ms": found.mtime_ms,
    })
}

/// Checks `options` before a search starts: an invalid path glob fails here.
pub(super) fn check_options(options: &SearchOptions) -> Result<(), String> {
    if options.glob && glob::is_path_glob(&options.query) {
        glob::PathGlob::parse(&options.query)
            .map_err(|error| format!("invalid glob `{}`: {error}", options.query))?;
    }
    Ok(())
}

/// The search of `options` below `target`, every hit to `sink`. Without a
/// path glob, `gitignore` and `sort` it is exactly `search_with` (hits as
/// they are found); otherwise hits are checked here, and with `sort` they
/// are collected, ordered and cut to `max_results` at the end. Reaching
/// `max_results` early sets `cancel` and ends with `Ok`.
pub(super) fn search_found(
    target: &Target,
    options: &SearchOptions,
    cancel: &AtomicBool,
    sink: &mut dyn FnMut(Found),
) -> Result<(), String> {
    let path_glob = if options.glob && glob::is_path_glob(&options.query) {
        Some(
            glob::PathGlob::parse(&options.query)
                .map_err(|error| format!("invalid glob `{}`: {error}", options.query))?,
        )
    } else {
        None
    };
    let checked = path_glob.is_some() || options.gitignore;
    if !checked && options.sort == SearchSort::Found {
        let spec = search_spec(
            &options.query,
            options.glob,
            options.max_results,
            options.want_dirs,
        );
        return search_with(target, &spec, cancel, sink);
    }
    let spec = match &path_glob {
        Some(path_glob) => match path_glob.name_hint() {
            Some(hint) => search_spec(&hint, true, 0, options.want_dirs),
            None => search_spec("", false, 0, options.want_dirs),
        },
        None => search_spec(&options.query, options.glob, 0, options.want_dirs),
    };
    let ignores = options
        .gitignore
        .then(|| ignore_files::BackendIgnores::new(&*target.backend, &target.path));
    let limit = options.max_results;
    let mut count = 0u64;
    let mut reached = false;
    let mut collected = Vec::new();
    let outcome = search_inner(
        target,
        &spec,
        cancel,
        ignores.as_ref(),
        &mut |found: Found| {
            if reached
                || path_glob
                    .as_ref()
                    .is_some_and(|path_glob| !path_glob.matches(&found.rel))
                || ignores.as_ref().is_some_and(|ignores| {
                    ignores.ignored(&found.rel, found.kind == EntryKind::Dir)
                })
            {
                return;
            }
            if options.sort != SearchSort::Found {
                collected.push(found);
                return;
            }
            count += 1;
            sink(found);
            if limit != 0 && count >= limit {
                reached = true;
                cancel.store(true, Ordering::Relaxed);
            }
        },
    );
    match outcome {
        Err(error) if !(reached && error == SEARCH_CANCELLED) => return Err(error),
        _ => {}
    }
    match options.sort {
        SearchSort::Found => {}
        SearchSort::Name => collected.sort_by(|a, b| a.rel.cmp(&b.rel)),
        SearchSort::Mtime => {
            collected.sort_by(|a, b| b.mtime_ms.cmp(&a.mtime_ms).then_with(|| a.rel.cmp(&b.rel)))
        }
    }
    if limit != 0 {
        collected.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    }
    for found in collected {
        sink(found);
    }
    Ok(())
}

/// `want_dirs` adds matching directories to the matching files.
pub(super) fn search_spec(
    query: &str,
    glob: bool,
    max_results: u64,
    want_dirs: bool,
) -> SearchSpec {
    SearchSpec {
        query: query.to_string(),
        glob,
        min_size: 0,
        max_size: 0,
        max_results,
        want_dirs,
    }
}

/// Streams every validated hit below `target` to `sink`: server-side search
/// where the backend has one, otherwise the bounded listing walk. Setting
/// `cancel` stops either path; the result is then an error.
pub(super) fn search_with(
    target: &Target,
    spec: &SearchSpec,
    cancel: &AtomicBool,
    sink: &mut dyn FnMut(Found),
) -> Result<(), String> {
    search_inner(target, spec, cancel, None, sink)
}

/// `search_with`; with `ignores` the walk does not enter ignored folders.
fn search_inner(
    target: &Target,
    spec: &SearchSpec,
    cancel: &AtomicBool,
    ignores: Option<&ignore_files::BackendIgnores<'_>>,
    sink: &mut dyn FnMut(Found),
) -> Result<(), String> {
    if target.backend.supports_search() {
        let (tx, rx) = crossbeam_channel::bounded(1024);
        let mut invalid = None;
        let mut received = false;
        let ran = std::thread::scope(|scope| {
            let handle = scope.spawn(|| target.backend.search(&target.path, spec, tx, cancel));
            for hit in rx {
                received = true;
                if invalid.is_some() {
                    continue;
                }
                if let Err(error) = crate::agent_proto::ValidatedRelativePath::parse(&hit.rel) {
                    cancel.store(true, Ordering::Relaxed);
                    invalid = Some(error.to_string());
                } else {
                    sink(Found::from_server(hit));
                }
            }
            handle
                .join()
                .map_err(|_| "server-side search worker panicked".to_string())
        })?;
        if let Some(error) = invalid {
            return Err(error);
        }
        let ran = ran.map_err(|error| error.to_string())?;
        if cancel.load(Ordering::Relaxed) {
            return Err(SEARCH_CANCELLED.to_string());
        }
        match ran {
            true => return Ok(()),
            false if received => {
                return Err(
                    "backend reported search unsupported after streaming results".to_string(),
                );
            }
            false => {}
        }
    }
    Walk {
        backend: &*target.backend,
        spec,
        count: 0,
        budget: SearchBudget::default(),
        sink,
        cancel,
        ignores,
    }
    .directory(&target.path, "", 0)
}

struct Walk<'a, 's> {
    backend: &'a dyn Backend,
    spec: &'a SearchSpec,
    count: u64,
    budget: SearchBudget,
    sink: &'a mut (dyn FnMut(Found) + 's),
    cancel: &'a AtomicBool,
    ignores: Option<&'a ignore_files::BackendIgnores<'a>>,
}

impl Walk<'_, '_> {
    fn directory(&mut self, dir: &str, rel_dir: &str, depth: usize) -> Result<(), String> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(SEARCH_CANCELLED.to_string());
        }
        self.budget.record(dir, depth)?;
        let mut names = std::collections::HashSet::new();
        // Tolerant: an entry whose name the listing cannot represent (U9)
        // is named on stderr; the rest of the folder is still searched.
        let listing = super::tree_guard::listing(self.backend, dir).map_err(|e| e.to_string())?;
        super::ops::report_omitted(dir, &listing.omitted);
        for child in listing.entries {
            if self.spec.max_results != 0 && self.count >= self.spec.max_results {
                break;
            }
            crate::vfs::validate_child_name(&child.name).map_err(|error| error.to_string())?;
            if !names.insert(child.name.clone()) {
                return Err(format!(
                    "backend returned duplicate child name in {dir}: {:?}",
                    child.name
                ));
            }
            let path = super::ops::join(dir, &child.name);
            self.budget.record(&path, depth + 1)?;
            let rel = if rel_dir.is_empty() {
                child.name.clone()
            } else {
                format!("{rel_dir}/{}", child.name)
            };
            let kind = EntryKind::of(&child);
            if matches_spec(&child, kind, self.spec) {
                self.count += 1;
                (self.sink)(Found {
                    rel: rel.clone(),
                    kind,
                    size: child.size,
                    mtime_ms: child.mtime_ms,
                });
            }
            // Only real directories are entered; a link is reported, not
            // followed; an ignored folder is not entered.
            if kind == EntryKind::Dir
                && !self
                    .ignores
                    .is_some_and(|ignores| ignores.ignored(&rel, true))
            {
                self.directory(&path, &rel, depth + 1)?;
            }
        }
        Ok(())
    }
}

fn matches_spec(entry: &VfsMeta, kind: EntryKind, spec: &SearchSpec) -> bool {
    let filtered = match kind {
        EntryKind::Dir => !spec.want_dirs,
        EntryKind::File => {
            (spec.max_size != 0 && entry.size > spec.max_size) || entry.size < spec.min_size
        }
        // A link's own size is the length of its target text and a special
        // file has no content size: the name decides.
        EntryKind::Link | EntryKind::Special => false,
    };
    if filtered {
        return false;
    }
    if spec.query.is_empty() {
        return true;
    }
    if spec.glob {
        glob_match(&spec.query, &entry.name)
    } else {
        entry
            .name
            .to_lowercase()
            .contains(&spec.query.to_lowercase())
    }
}

pub(super) fn glob_match(pat: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (
        pat.to_lowercase().chars().collect(),
        text.to_lowercase().chars().collect(),
    );
    let (mut pi, mut ti, mut star, mut mark) = (0usize, 0usize, usize::MAX, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = pi;
            mark = ti;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::glob_match;

    #[test]
    fn glob_matches_case_insensitive() {
        assert!(glob_match("*.TXT", "note.txt"));
        assert!(!glob_match("a?.txt", "abc.txt"));
    }
}
