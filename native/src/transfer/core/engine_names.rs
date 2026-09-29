//! Destination names of a transfer job: where each selected entry lands below
//! the target folder, numbered names for new top-level entries at remote
//! targets, and the validation of explicit relative paths. Pure logic on
//! forward-slash strings.
use crate::vfs::remote_util::{numbered_remote_name, REMOTE_UNIQUE_ATTEMPTS};
use std::collections::HashSet;

/// The last component of a forward-slash path.
pub(crate) fn base_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

/// The folder holding `path`; top-level entries resolve to their root
/// (`/`, or `C:/` for a drive).
pub(crate) fn parent_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rsplit_once('/') {
        Some(("", _)) | None => "/".to_string(),
        Some((parent, _)) if parent.ends_with(':') => format!("{parent}/"),
        Some((parent, _)) => parent.to_string(),
    }
}

/// The folder part of a relative path (`None` for a top-level entry).
pub(crate) fn parent_rel(rel: &str) -> Option<&str> {
    rel.rsplit_once('/').map(|(parent, _)| parent)
}

/// The top-level component of a relative path.
pub(crate) fn first_component(rel: &str) -> &str {
    rel.split('/').next().unwrap_or(rel)
}

/// `rel` with its top-level component replaced by `first`.
pub(crate) fn with_first(rel: &str, first: &str) -> String {
    match rel.split_once('/') {
        Some((_, rest)) => format!("{first}/{rest}"),
        None => first.to_string(),
    }
}

pub(crate) fn join_rel(base: &str, rel: &str) -> String {
    if rel.is_empty() {
        return base.to_string();
    }
    format!("{}/{}", base.trim_end_matches('/'), rel)
}

/// Where a selected entry lands: its path below `base` (component-bounded;
/// the copy dialog and recursive views keep that structure), else its name.
pub(crate) fn root_rel(path: &str, base: Option<&str>) -> String {
    let name = base_name(path).to_string();
    let Some(base) = base else {
        return name;
    };
    let base = base.trim_end_matches('/');
    let path = path.trim_end_matches('/');
    let relative = if base.is_empty() {
        Some(path.trim_start_matches('/'))
    } else {
        path.strip_prefix(base)
            .and_then(|rest| rest.strip_prefix('/'))
    };
    match relative {
        Some(relative) if !relative.is_empty() => relative.to_string(),
        _ => name,
    }
}

/// Why a name or relative path cannot be created below a target folder.
pub(crate) fn name_problem(name: &str, allow_backslash: bool) -> Option<String> {
    let reserved = name.is_empty()
        || matches!(name, "." | "..")
        || name.contains(['/', '\0'])
        || (!allow_backslash && name.contains('\\'));
    reserved.then(|| format!("Ungültiger Name {name:?}"))
}

/// Validates explicit relative destinations (filtered clipboard pairs): no
/// empty, `.`/`..`, drive-like or otherwise unsafe components, no destination
/// twice and no destination that is also the folder of another one. The
/// whole set is refused before anything is created.
pub(crate) fn validate_pair_rels<'a>(
    rels: impl IntoIterator<Item = &'a str>,
    allow_backslash: bool,
) -> Result<(), String> {
    let mut files: HashSet<&str> = HashSet::new();
    let mut folders: HashSet<&str> = HashSet::new();
    for rel in rels {
        let invalid = || format!("Ungültiger relativer Zielpfad: {rel}");
        if rel.is_empty() || rel.starts_with('/') {
            return Err(invalid());
        }
        for component in rel.split('/') {
            if component.contains(':') || name_problem(component, allow_backslash).is_some() {
                return Err(invalid());
            }
        }
        if folders.contains(rel) || !files.insert(rel) {
            return Err(format!("Mehrdeutiges Dateiziel: {rel}"));
        }
        let mut parent = rel;
        while let Some(folder) = parent_rel(parent) {
            if files.contains(folder) {
                return Err(format!("Ziel ist zugleich Datei und Ordner: {folder}"));
            }
            folders.insert(folder);
            parent = folder;
        }
    }
    Ok(())
}

/// The names after `name` in the "Name (n)" sequence it belongs to:
/// `a.txt` → `a (2).txt`, `a (3).txt` …; `a (2).txt` → `a (3).txt` ….
pub(crate) fn next_numbered(name: &str) -> impl Iterator<Item = String> {
    let (stem, extension) = match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name, ""),
    };
    let numbered = stem
        .strip_suffix(')')
        .and_then(|rest| rest.rsplit_once(" ("))
        .and_then(|(base, digits)| {
            let index = digits.parse::<usize>().ok()?;
            (index >= 2 && !base.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
                .then_some((base, index))
        });
    let (base, start) = numbered.unwrap_or((stem, 1));
    let base = format!("{base}{extension}");
    (start + 1..=REMOTE_UNIQUE_ATTEMPTS).map(move |index| numbered_remote_name(&base, index))
}

/// Chooses free names for new top-level entries at a remote target from one
/// listing of the target folder: `name`, `name (2)`, … never one that exists
/// or was chosen before in the same job.
pub(crate) struct NamePlanner {
    taken: HashSet<String>,
    case_sensitive: bool,
}

impl NamePlanner {
    pub(crate) fn new(existing: impl IntoIterator<Item = String>, case_sensitive: bool) -> Self {
        let mut planner = Self {
            taken: HashSet::new(),
            case_sensitive,
        };
        for name in existing {
            let key = planner.key(&name);
            planner.taken.insert(key);
        }
        planner
    }

    fn key(&self, name: &str) -> String {
        if self.case_sensitive {
            name.to_string()
        } else {
            name.to_lowercase()
        }
    }

    /// A free name for `wanted`; `None` when every numbered name is taken.
    pub(crate) fn claim(&mut self, wanted: &str) -> Option<String> {
        (1..=REMOTE_UNIQUE_ATTEMPTS)
            .map(|index| numbered_remote_name(wanted, index))
            .find(|candidate| {
                let key = self.key(candidate);
                self.taken.insert(key)
            })
    }

    /// `taken` turned out to exist after all (created since the listing):
    /// the next free name for `wanted`.
    pub(crate) fn reclaim(&mut self, wanted: &str, taken: &str) -> Option<String> {
        let key = self.key(taken);
        self.taken.insert(key);
        self.claim(wanted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_engine_task_names_root_rel_keeps_structure_below_base() {
        assert_eq!(root_rel("/r/A", None), "A");
        assert_eq!(root_rel("/r/A/x.txt", Some("/r")), "A/x.txt");
        assert_eq!(root_rel("/r/A/x.txt", Some("/r/")), "A/x.txt");
        assert_eq!(root_rel("/rooted/a.txt", Some("/r")), "a.txt");
        assert_eq!(root_rel("/r", Some("/r")), "r");
        assert_eq!(root_rel("C:/Data/f", Some("C:/Data")), "f");
        assert_eq!(with_first("A/b/c", "A (2)"), "A (2)/b/c");
        assert_eq!(with_first("f.txt", "f (2).txt"), "f (2).txt");
        assert_eq!(parent_rel("a/b/c"), Some("a/b"));
        assert_eq!(parent_rel("a"), None);
        assert_eq!(first_component("a/b"), "a");
    }

    #[test]
    fn transfer_engine_task_names_planner_numbers_taken_names() {
        let mut planner = NamePlanner::new(["Vault".to_string(), "note.txt".into()], false);
        assert_eq!(planner.claim("vault").as_deref(), Some("vault (2)"));
        assert_eq!(planner.claim("vault").as_deref(), Some("vault (3)"));
        assert_eq!(planner.claim("note.txt").as_deref(), Some("note (2).txt"));
        assert_eq!(planner.claim("new").as_deref(), Some("new"));
        assert_eq!(
            planner.reclaim("fresh", "fresh").as_deref(),
            Some("fresh (2)")
        );
        let mut exact = NamePlanner::new(["Vault".to_string()], true);
        assert_eq!(exact.claim("vault").as_deref(), Some("vault"));
        let next: Vec<String> = next_numbered("a.txt").take(2).collect();
        assert_eq!(next, ["a (2).txt", "a (3).txt"]);
        assert_eq!(
            next_numbered("a (2).txt").next().as_deref(),
            Some("a (3).txt")
        );
        assert_eq!(
            next_numbered("Ordner (9)").next().as_deref(),
            Some("Ordner (10)")
        );
        assert_eq!(
            next_numbered("x (y).txt").next().as_deref(),
            Some("x (y) (2).txt")
        );
        assert_eq!(
            next_numbered(".profile").next().as_deref(),
            Some(".profile (2)")
        );
    }

    #[test]
    fn transfer_engine_task_names_pair_sets_are_validated_as_a_whole() {
        assert!(validate_pair_rels(["Vault/子/one.md", "Vault/two.md"], false).is_ok());
        let invalid: [&[&str]; 10] = [
            &["../escape"],
            &["/absolute"],
            &["C:/drive"],
            &["a\\b"],
            &["a//b"],
            &["a/./b"],
            &["a\0b"],
            &["a", "a/b"],
            &["a/b", "a"],
            &["same", "same"],
        ];
        for set in invalid {
            assert!(
                validate_pair_rels(set.iter().copied(), false).is_err(),
                "{set:?}"
            );
        }
        assert!(validate_pair_rels(["a\\b"], true).is_ok());
    }
}
