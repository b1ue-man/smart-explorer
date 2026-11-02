//! Overlap of two backend-relative location paths (`/Docs` and `/Docs/a`),
//! after `.`/`..` segments and trailing slashes are resolved literally. Pure
//! string rules, shared by the sync root checks of `vfs` and `syncjobs` and
//! by `connect` (`connect::location_paths_overlap` re-exports it, D13).

/// Whether `a` and `b` are the same path or one lies inside the other.
pub fn paths_overlap(a: &str, b: &str) -> bool {
    let (a, b) = (path_key(a), path_key(b));
    paths_overlap_one_way(&a, &b) || paths_overlap_one_way(&b, &a)
}

/// Whether `child` is `parent` or lies inside it (both already keys).
pub fn paths_overlap_one_way(parent: &str, child: &str) -> bool {
    parent == child
        || child
            .strip_prefix(parent)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The comparison key of a path: `.` dropped, `..` applied to a named
/// parent, no trailing slash.
pub fn path_key(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "." => {}
            ".." if parts
                .last()
                .is_some_and(|part| !part.is_empty() && *part != "..") =>
            {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/").trim_end_matches('/').to_string()
}
