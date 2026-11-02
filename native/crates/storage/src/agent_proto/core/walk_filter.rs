//! The sync job's filter a walk next to the data applies (agent and service
//! `walk-filter-v1`, Share `hash_walk_filter_v1`), and how the walk reads
//! digests (`fresh`, the checksum compare mode).
//!
//! The filter is the client's own (`bisync::WalkFilter`): hidden entries,
//! the ignore patterns the walk can evaluate with the client's meaning
//! (`walk_pattern.rs`; the client keeps applying every pattern), size and
//! time bounds. What it leaves out the walk still sends as an entry, so the
//! client sees every entry it saw before, files the filter leaves out
//! included (the planner keeps them as filtered files, they protect their
//! folders and never read as deletions). The walk only does less for them:
//! - a regular file the filter leaves out gets no digest;
//! - a folder the filter leaves out is not entered (its entry comes, its
//!   content does not; a keyed walk gives it no key, `folder_key.rs`).
//!
//! The rules are those of the client walk (`bisync::snapshot_dir`
//! `scan_listing`, `snapshot_host_walk` `never_entered`): a file is left out
//! when it is hidden and hidden entries are skipped, when a pattern matches
//! its path, or when its size or time lies outside the bounds; a folder when
//! it is hidden and hidden entries are skipped, or when a pattern matches its
//! path or its path with a trailing `/`. Paths are relative to the walk root
//! and `/`-separated; with `backslash_separates` (a Windows client, as
//! globset's candidate) a `\` in a name is matched as `/`. So the walk never
//! leaves out more than the client does: an entry the client takes always
//! arrives with its digest and its content.
use std::io;

use super::walk_pattern::{pattern_states, PatternSet, MAX_STATES};

/// Patterns one filter carries at most (`syncjobs` allows 4096).
pub const MAX_FILTER_PATTERNS: usize = 4096;
/// Text of all patterns of one filter.
pub const MAX_FILTER_TEXT: usize = 1024 * 1024;
/// JSON text the patterns of one filter take at most (`pattern_wire_cost`
/// summed): what a Share request carries beside its largest known-key text
/// or folder list (`WalkFilterSpec::bounded`). `host_patterns`
/// keeps every job within it, so the agent, the Share host and a local side
/// all apply the same list and equal folders get equal keys everywhere.
pub const HOST_PATTERN_WIRE: usize = 16 * 1024;

/// The JSON text of `pattern` as one string of a list (with its comma), as
/// `serde_json` writes it: quotes, `"` `\` and the short control escapes as
/// two bytes, every other control character as six, the rest as its UTF-8.
pub fn pattern_wire_cost(pattern: &str) -> usize {
    let escaped: usize = pattern
        .chars()
        .map(|c| match c {
            '"' | '\\' | '\u{8}' | '\u{c}' | '\n' | '\r' | '\t' => 2,
            c if (c as u32) < 0x20 => 6,
            c => c.len_utf8(),
        })
        .sum();
    escaped + 3
}

/// The filter as the client sends it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WalkFilterSpec {
    /// Hidden entries are left out (the job does not include them).
    pub skip_hidden: bool,
    /// Ignore patterns as globset regular expressions (`walk_pattern.rs`).
    pub patterns: Vec<String>,
    /// A `\` in a path is matched as `/` (globset's candidate on Windows).
    pub backslash_separates: bool,
    /// Bounds of regular files (bytes, Unix ms); 0 = no bound.
    pub min_size: u64,
    pub max_size: u64,
    pub after_mtime_ms: i64,
    pub before_mtime_ms: i64,
}

impl WalkFilterSpec {
    /// Whether the filter leaves nothing out.
    pub fn is_empty(&self) -> bool {
        !self.skip_hidden
            && self.patterns.is_empty()
            && self.min_size == 0
            && self.max_size == 0
            && self.after_mtime_ms == 0
            && self.before_mtime_ms == 0
    }

    /// `self` with its patterns, in order, as far as they fit
    /// `HOST_PATTERN_WIRE` (what a Share request carries). A pattern left
    /// out is applied by the client alone: the host then leaves out less,
    /// never more. A list from `host_patterns` always fits whole.
    pub fn bounded(&self) -> WalkFilterSpec {
        let mut used = 0usize;
        let mut patterns = Vec::new();
        for pattern in &self.patterns {
            let cost = pattern_wire_cost(pattern);
            if used.saturating_add(cost) > HOST_PATTERN_WIRE {
                break;
            }
            used += cost;
            patterns.push(pattern.clone());
        }
        WalkFilterSpec {
            patterns,
            ..self.clone()
        }
    }

    /// The filter ready to apply; refused when a pattern or the whole set
    /// exceeds what the walk evaluates (the client then sends none).
    pub fn compile(&self) -> io::Result<HostFilter> {
        if self.patterns.len() > MAX_FILTER_PATTERNS
            || self.patterns.iter().map(String::len).sum::<usize>() > MAX_FILTER_TEXT
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "walk filter too large",
            ));
        }
        Ok(HostFilter {
            skip_hidden: self.skip_hidden,
            patterns: PatternSet::new(&self.patterns)?,
            backslash_separates: self.backslash_separates,
            min_size: self.min_size,
            max_size: self.max_size,
            after_mtime_ms: self.after_mtime_ms,
            before_mtime_ms: self.before_mtime_ms,
        })
    }
}

/// The patterns of `regexes` (globset regular expressions of one job, in
/// order) the walk next to the data evaluates with the client's meaning,
/// within the limits of one filter and the text every host takes
/// (`HOST_PATTERN_WIRE`): one list for the agent, the Share host and the
/// local side's keys. A pattern left out here is applied by the client alone.
pub fn host_patterns(regexes: Vec<String>) -> Vec<String> {
    let mut accepted = Vec::new();
    // The shared match state plus one split per pattern after the first.
    let mut states = 1usize;
    let mut text = 0usize;
    let mut wire = 0usize;
    for regex in regexes {
        let Ok(own) = pattern_states(&regex) else {
            continue;
        };
        // Its states without its own match state, plus one split.
        let cost = own;
        let wire_cost = pattern_wire_cost(&regex);
        if accepted.len() >= MAX_FILTER_PATTERNS
            || states.saturating_add(cost) > MAX_STATES
            || text.saturating_add(regex.len()) > MAX_FILTER_TEXT
            || wire.saturating_add(wire_cost) > HOST_PATTERN_WIRE
        {
            continue;
        }
        states += cost;
        text += regex.len();
        wire += wire_cost;
        accepted.push(regex);
    }
    accepted
}

/// What a walk next to the data takes from the sync: the digests read now,
/// and the filter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WalkScope {
    /// Every digest is read from the file now (the checksum compare mode):
    /// the digest cache of the host is not asked; the digest read may be
    /// stored.
    pub fresh: bool,
    pub filter: Option<WalkFilterSpec>,
}

impl WalkScope {
    pub fn is_default(&self) -> bool {
        !self.fresh && self.filter.is_none()
    }
}

/// A compiled filter (`WalkFilterSpec::compile`).
#[derive(Debug)]
pub struct HostFilter {
    skip_hidden: bool,
    patterns: PatternSet,
    backslash_separates: bool,
    min_size: u64,
    max_size: u64,
    after_mtime_ms: i64,
    before_mtime_ms: i64,
}

impl HostFilter {
    /// The regular file `rel` (path below the walk root) gets no digest.
    pub fn leaves_out_file(&self, rel: &str, hidden: bool, size: u64, mtime_ms: i64) -> bool {
        (self.skip_hidden && hidden)
            || self.ignored(rel, false)
            || !self.bounds_take(size, mtime_ms)
    }

    /// The folder `rel` is not entered.
    pub fn leaves_out_dir(&self, rel: &str, hidden: bool) -> bool {
        (self.skip_hidden && hidden) || self.ignored(rel, true)
    }

    /// `bisync::WalkFilter::ignored`: the path, and for a folder also the
    /// path with a trailing `/`.
    fn ignored(&self, rel: &str, folder: bool) -> bool {
        if self.patterns.is_empty() {
            return false;
        }
        let mut path = rel.as_bytes().to_vec();
        if self.backslash_separates {
            for byte in &mut path {
                if *byte == b'\\' {
                    *byte = b'/';
                }
            }
        }
        if self.patterns.is_match(&path) {
            return true;
        }
        if !folder {
            return false;
        }
        path.push(b'/');
        self.patterns.is_match(&path)
    }

    /// `bisync::WalkFilter::size_age_ok`.
    fn bounds_take(&self, size: u64, mtime_ms: i64) -> bool {
        !((self.min_size > 0 && size < self.min_size)
            || (self.max_size > 0 && size > self.max_size)
            || (self.after_mtime_ms > 0 && mtime_ms < self.after_mtime_ms)
            || (self.before_mtime_ms > 0 && mtime_ms > self.before_mtime_ms))
    }
}

/// The filter of `scope`, compiled; `None` without one.
pub fn compiled(scope: &WalkScope) -> io::Result<Option<HostFilter>> {
    scope
        .filter
        .as_ref()
        .map(WalkFilterSpec::compile)
        .transpose()
}

#[cfg(test)]
#[path = "walk_filter_tests.rs"]
mod tests;
