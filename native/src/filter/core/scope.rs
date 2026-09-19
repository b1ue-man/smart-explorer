//! Filter scope for pruned recursive scans. A recursive scan that ran with a
//! retention filter kept only the entries matching the filter active at its
//! start (plus the directories needed to show them). A later filter may
//! refilter those retained entries only when it is provably at least as
//! narrow; anything else has to restart the scan. The comparison treats a
//! filter as a conjunction of independent constraints and stays conservative:
//! whatever it cannot prove counts as broader.
use super::imp::text_groups;
use super::CompiledFilter;
use super::extensions::{normalize_extensions, suffix_matches};
use crate::scanner::ScanRetention;
use crate::types::{FileEntry, FilterDef, Range, TextMode};

/// True when `filter` rejects at least one kind of entry a pass-all filter
/// would show, i.e. when a recursive scan can prune with it.
pub fn filter_prunes(filter: &FilterDef) -> bool {
    !filter.text.trim().is_empty()
        || !filter.extensions.is_empty()
        || filter.size.min.is_some()
        || filter.size.max.is_some()
        || filter.mtime.min.is_some()
        || filter.mtime.max.is_some()
        || filter.btime.min.is_some()
        || filter.btime.max.is_some()
        || filter.depth.min.is_some()
        || filter.depth.max.is_some()
        || !filter.include_files
        || !filter.include_dirs
        || !filter.include_hidden
        || !filter.include_system
        || filter.problem_names_only
}

/// True when every entry `current` accepts is also accepted by `scanned_with`,
/// so a listing pruned with `scanned_with` already contains everything
/// `current` could show.
pub fn filter_is_at_least_as_narrow(current: &FilterDef, scanned_with: &FilterDef) -> bool {
    if CompiledFilter::compile(scanned_with).error().is_some() {
        return CompiledFilter::compile(current).error().is_some();
    }
    text_at_least_as_narrow(current, scanned_with)
        && extensions_at_least_as_narrow(&current.extensions, &scanned_with.extensions)
        && range_within(&current.size, &scanned_with.size)
        && range_within(&current.mtime, &scanned_with.mtime)
        && range_within(&current.btime, &scanned_with.btime)
        && range_within(&current.depth, &scanned_with.depth)
        && implies(current.include_files, scanned_with.include_files)
        && implies(current.include_dirs, scanned_with.include_dirs)
        && implies(current.include_hidden, scanned_with.include_hidden)
        && implies(current.include_system, scanned_with.include_system)
        && implies(scanned_with.problem_names_only, current.problem_names_only)
}

/// Whether a filter change needs a new recursive scan. `scanned_with` is the
/// filter the current listing was pruned with (`None` = unpruned) and
/// `truncated` tells whether the bounded budget cut that scan short: then any
/// change that could let a pruned rescan finish is worth restarting for.
pub fn scan_restart_needed(
    scanned_with: Option<&FilterDef>,
    truncated: bool,
    current: &FilterDef,
) -> bool {
    match scanned_with {
        None => truncated && filter_prunes(current),
        Some(previous) => {
            !filter_is_at_least_as_narrow(current, previous) || (truncated && current != previous)
        }
    }
}

fn implies(premise: bool, conclusion: bool) -> bool {
    !premise || conclusion
}

fn range_within<T: PartialOrd + Copy>(current: &Range<T>, scanned: &Range<T>) -> bool {
    let min_ok = match (scanned.min, current.min) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(scanned), Some(current)) => current >= scanned,
    };
    let max_ok = match (scanned.max, current.max) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(scanned), Some(current)) => current <= scanned,
    };
    min_ok && max_ok
}

fn extensions_at_least_as_narrow(current: &[String], scanned: &[String]) -> bool {
    let scanned = normalize_extensions(scanned);
    if scanned.is_empty() {
        return true;
    }
    let current = normalize_extensions(current);
    !current.is_empty()
        && current.iter().all(|extension| {
            scanned.iter().any(|old| old == extension || suffix_matches(extension, old))
        })
}

fn text_at_least_as_narrow(current: &FilterDef, scanned: &FilterDef) -> bool {
    let scanned_text = scanned.text.trim();
    if scanned_text.is_empty() {
        return true;
    }
    let current_text = current.text.trim();
    if current.text_mode != scanned.text_mode {
        return false;
    }
    match current.text_mode {
        TextMode::Substring => substring_narrows(current_text, scanned_text),
        TextMode::Regex | TextMode::Glob => current_text == scanned_text,
    }
}

/// `name.contains(term)` semantics: a current OR-group narrows one scanned
/// OR-group when every scanned AND-term is contained in some current AND-term
/// of that group; every current group must narrow some scanned group.
fn substring_narrows(current: &str, scanned: &str) -> bool {
    let scanned_groups = text_groups(scanned);
    if scanned_groups.is_empty() {
        return true;
    }
    let current_groups = text_groups(current);
    !current_groups.is_empty()
        && current_groups.iter().all(|group| {
            scanned_groups.iter().any(|old| {
                old.iter()
                    .all(|term| group.iter().any(|newer| newer.contains(term.as_str())))
            })
        })
}

/// Scan-time retention for one filter: files and directories are kept when
/// they match the filter; a directory is descended only while its descendants
/// could still appear in the tree view (directory flags, depth ceiling).
pub struct FilterRetention {
    compiled: CompiledFilter,
    filter: FilterDef,
    root_prefix: String,
}

impl FilterRetention {
    pub fn new(filter: FilterDef, root_prefix: String) -> Self {
        Self {
            compiled: CompiledFilter::compile(&filter),
            filter,
            root_prefix,
        }
    }

    /// The depth ceiling the walker can stop at: entries below `depth.max`
    /// can never match.
    pub fn max_depth(&self) -> Option<u32> {
        self.filter.depth.max
    }
}

impl ScanRetention for FilterRetention {
    fn retain(&self, entry: &FileEntry) -> bool {
        self.compiled.matches(entry, &self.root_prefix)
    }

    fn descend(&self, directory: &FileEntry) -> bool {
        self.compiled.error().is_none()
            && (!directory.hidden || self.filter.include_hidden)
            && (!directory.system || self.filter.include_system)
            && self
                .filter
                .depth
                .max
                .is_none_or(|maximum| directory.depth < maximum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn with_text(text: &str) -> FilterDef {
        let mut filter = FilterDef::new();
        filter.text = text.to_string();
        filter
    }

    fn entry(name: &str, is_dir: bool, depth: u32) -> FileEntry {
        FileEntry {
            path: Arc::from(format!("/root/{name}")),
            parent: Arc::from("/root"),
            name: Arc::from(name),
            ext: Arc::from(name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("")),
            size: 10,
            mtime_ms: 0,
            btime_ms: 0,
            is_dir,
            is_symlink: false,
            hidden: false,
            system: false,
            depth,
            id: None,
        }
    }

    #[test]
    fn recursive_filter_task_pass_all_filter_never_prunes() {
        assert!(!filter_prunes(&FilterDef::new()));
        assert!(filter_prunes(&with_text("a")));
        let mut names = FilterDef::new();
        names.problem_names_only = true;
        assert!(filter_prunes(&names));
        let mut hidden = FilterDef::new();
        hidden.include_hidden = false;
        assert!(filter_prunes(&hidden));
    }

    #[test]
    fn recursive_filter_task_substring_text_narrows_by_containment() {
        assert!(filter_is_at_least_as_narrow(
            &with_text("invoice"),
            &with_text("inv")
        ));
        assert!(filter_is_at_least_as_narrow(
            &with_text("  Invoice "),
            &with_text("invoice")
        ));
        assert!(filter_is_at_least_as_narrow(
            &with_text("invoice, 2024"),
            &with_text("invoice")
        ));
        assert!(filter_is_at_least_as_narrow(
            &with_text("invoice"),
            &with_text("invoice; receipt")
        ));
        assert!(filter_is_at_least_as_narrow(
            &with_text("anything"),
            &FilterDef::new()
        ));
        assert!(!filter_is_at_least_as_narrow(
            &with_text("inv"),
            &with_text("invoice")
        ));
        assert!(!filter_is_at_least_as_narrow(
            &with_text("invoice; receipt"),
            &with_text("invoice")
        ));
        assert!(!filter_is_at_least_as_narrow(
            &FilterDef::new(),
            &with_text("invoice")
        ));
        let mut regex = with_text("inv.*");
        regex.text_mode = TextMode::Regex;
        assert!(filter_is_at_least_as_narrow(&regex, &regex));
        assert!(!filter_is_at_least_as_narrow(&regex, &with_text("inv")));
        let mut longer_regex = with_text("inv.*ce");
        longer_regex.text_mode = TextMode::Regex;
        assert!(!filter_is_at_least_as_narrow(&longer_regex, &regex));
    }

    #[test]
    fn recursive_filter_task_structured_constraints_narrow_by_containment() {
        let mut scanned = FilterDef::new();
        scanned.extensions = vec!["jpg".into(), "png".into()];
        scanned.size.min = Some(100);
        scanned.mtime.max = Some(50);
        scanned.include_hidden = false;

        let mut narrower = scanned.clone();
        narrower.extensions = vec![".PNG".into()];
        narrower.size.min = Some(200);
        narrower.size.max = Some(900);
        narrower.mtime.max = Some(40);
        narrower.include_system = false;
        narrower.problem_names_only = true;
        assert!(filter_is_at_least_as_narrow(&narrower, &scanned));
        assert!(filter_is_at_least_as_narrow(&scanned, &scanned));

        let mut more_extensions = scanned.clone();
        more_extensions.extensions.push("gif".into());
        assert!(!filter_is_at_least_as_narrow(&more_extensions, &scanned));
        let mut no_extensions = scanned.clone();
        no_extensions.extensions.clear();
        assert!(!filter_is_at_least_as_narrow(&no_extensions, &scanned));
        let mut smaller_min = scanned.clone();
        smaller_min.size.min = Some(50);
        assert!(!filter_is_at_least_as_narrow(&smaller_min, &scanned));
        let mut no_min = scanned.clone();
        no_min.size.min = None;
        assert!(!filter_is_at_least_as_narrow(&no_min, &scanned));
        let mut hidden_again = scanned.clone();
        hidden_again.include_hidden = true;
        assert!(!filter_is_at_least_as_narrow(&hidden_again, &scanned));
        let mut names = FilterDef::new();
        names.problem_names_only = true;
        assert!(!filter_is_at_least_as_narrow(&FilterDef::new(), &names));
    }

    #[test]
    fn recursive_filter_task_restart_only_for_broader_or_truncated_listings() {
        let unpruned: Option<&FilterDef> = None;
        assert!(!scan_restart_needed(unpruned, false, &with_text("a")));
        assert!(!scan_restart_needed(unpruned, true, &FilterDef::new()));
        assert!(scan_restart_needed(unpruned, true, &with_text("a")));

        let pruned = with_text("inv");
        assert!(!scan_restart_needed(
            Some(&pruned),
            false,
            &with_text("invoice")
        ));
        assert!(!scan_restart_needed(Some(&pruned), false, &pruned));
        assert!(scan_restart_needed(Some(&pruned), false, &with_text("in")));
        assert!(scan_restart_needed(Some(&pruned), false, &FilterDef::new()));
        assert!(scan_restart_needed(
            Some(&pruned),
            true,
            &with_text("invoice")
        ));
        assert!(!scan_restart_needed(Some(&pruned), true, &pruned));
    }

    #[test]
    fn recursive_filter_task_filter_retention_keeps_matches_and_descends_within_view() {
        let mut filter = with_text("report");
        filter.depth.max = Some(2);
        filter.include_hidden = false;
        let retention = FilterRetention::new(filter, "/root".into());
        assert_eq!(retention.max_depth(), Some(2));
        assert!(retention.retain(&entry("report.txt", false, 1)));
        assert!(!retention.retain(&entry("other.txt", false, 1)));
        assert!(retention.retain(&entry("reports", true, 1)));
        assert!(retention.descend(&entry("sub", true, 1)));
        assert!(!retention.descend(&entry("deep", true, 2)));
        let mut hidden = entry("hidden", true, 1);
        hidden.hidden = true;
        assert!(!retention.descend(&hidden));

        let mut files_only = FilterDef::new();
        files_only.include_dirs = false;
        let retention = FilterRetention::new(files_only, "/root".into());
        assert!(!retention.descend(&entry("sub", true, 1)));
    }
}
