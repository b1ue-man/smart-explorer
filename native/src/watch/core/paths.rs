//! Path helpers of the watch service (platform-neutral): relative paths with
//! `/` separators, ancestor prefixes for the consumer filter, the mapping of
//! host-reported changes to watch roots and the own-directory exclusion.

use std::path::{Component, Path};

/// `dir/name` with `/` separators; `name` alone below the root.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn join_rel(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// The directory prefixes of `rel` ("a", "a/b" for "a/b/c"), outermost first.
pub(crate) fn ancestors(rel: &str) -> impl Iterator<Item = &str> {
    rel.match_indices('/').map(move |(index, _)| &rel[..index])
}

/// `path` relative to `root` with `/` separators (`Some("")` for the root
/// itself), `None` when it lies outside.
pub(crate) fn rel_below(root: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(root).ok()?;
    let mut rel = String::new();
    for component in rest.components() {
        match component {
            Component::Normal(name) => {
                if !rel.is_empty() {
                    rel.push('/');
                }
                rel.push_str(&name.to_string_lossy());
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(rel)
}

/// Which part of a watch root a host-reported change at `changed` touches:
/// the path below the root when it lies inside, the whole root (`""`) when
/// the root lies below the changed path, `None` otherwise.
pub(crate) fn host_change_rel(root: &Path, changed: &Path) -> Option<String> {
    if let Some(rel) = rel_below(root, changed) {
        return Some(rel);
    }
    root.starts_with(changed).then(String::new)
}

/// Whether `path` is one of the app's own directories or lies below one.
pub(crate) fn is_own(path: &Path, own: &[std::path::PathBuf]) -> bool {
    own.iter().any(|dir| path.starts_with(dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn review_task_watch_paths_join_and_list_ancestors() {
        assert_eq!(join_rel("", "a"), "a");
        assert_eq!(join_rel("a/b", "c"), "a/b/c");
        assert_eq!(ancestors("a/b/c").collect::<Vec<_>>(), ["a", "a/b"]);
        assert_eq!(ancestors("file").count(), 0);
    }

    #[test]
    fn review_task_watch_host_changes_map_to_roots() {
        let root = PathBuf::from("/storage/emulated/0/DCIM");
        assert_eq!(
            host_change_rel(&root, Path::new("/storage/emulated/0/DCIM/Camera/a.jpg")),
            Some("Camera/a.jpg".into())
        );
        assert_eq!(
            host_change_rel(&root, Path::new("/storage/emulated/0")),
            Some(String::new())
        );
        assert_eq!(
            host_change_rel(&root, Path::new("/storage/emulated/0/Music")),
            None
        );
        assert_eq!(
            host_change_rel(&root, Path::new("/storage/emulated/0/DCIMX")),
            None
        );
    }

    #[test]
    fn review_task_watch_own_directories_are_recognised() {
        let own = [PathBuf::from("/home/u/.local/share/smart_explorer")];
        assert!(is_own(
            Path::new("/home/u/.local/share/smart_explorer/sync/daemon.heartbeat"),
            &own
        ));
        assert!(!is_own(Path::new("/home/u/.local/share/other"), &own));
    }
}
