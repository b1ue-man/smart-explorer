use super::engine::{invalid_data, MountEngine};
use super::path::ProjectedPath;
use super::windows_case::windows_ordinal_key;
use std::io;

pub(super) fn identity_key(case_sensitive: bool, path: &str) -> String {
    if case_sensitive {
        path.to_string()
    } else {
        windows_ordinal_key(path)
    }
}

pub(super) fn validate_backend_case_path(
    case_sensitive: bool,
    root: &str,
    path: &str,
) -> io::Result<()> {
    if case_sensitive {
        return Ok(());
    }
    for component in relative_components(root, path)? {
        super::windows_case::validate_windows_case_component(component)?;
    }
    Ok(())
}

impl MountEngine {
    /// True only when the exact exported backend root guarantees distinct
    /// identities for names that differ solely by letter case.
    pub fn case_sensitive_paths(&self) -> bool {
        self.case_sensitive_paths
    }

    pub(super) fn cache_key(&self, path: &str) -> String {
        identity_key(self.case_sensitive_paths, path)
    }

    pub(super) fn name_key(&self, name: &str) -> String {
        identity_key(self.case_sensitive_paths, name)
    }

    pub(super) fn validate_projected_case(&self, path: &ProjectedPath) -> io::Result<()> {
        if self.case_sensitive_paths || path.relative().is_empty() {
            return Ok(());
        }
        for component in path.relative().split('/') {
            super::windows_case::validate_windows_case_component(component)?;
        }
        Ok(())
    }

    pub(super) fn paths_equal(&self, left: &str, right: &str) -> bool {
        left == right
            || (!self.case_sensitive_paths
                && windows_ordinal_key(left) == windows_ordinal_key(right))
    }

    /// Returns the original-cased suffix when `path` is `ancestor` or lies
    /// below it. Searching original slash boundaries avoids indexing a Unicode
    /// string by the byte length of its folded representation.
    pub(super) fn descendant_suffix<'a>(&self, path: &'a str, ancestor: &str) -> Option<&'a str> {
        if self.paths_equal(path, ancestor) {
            return Some("");
        }
        path.char_indices()
            .filter(|(_, character)| *character == '/')
            .find_map(|(index, _)| {
                (index > 0 && self.paths_equal(&path[..index], ancestor)).then_some(&path[index..])
            })
    }

    pub(super) fn is_descendant(&self, path: &str, ancestor: &str) -> bool {
        self.descendant_suffix(path, ancestor)
            .is_some_and(|suffix| !suffix.is_empty())
    }

    /// A backend that did not prove case sensitivity may still physically
    /// contain two case-colliding names (for example an unclassified Unix
    /// SFTP server). Before treating differently-cased opens as one cached
    /// object, require the parent listing to identify exactly that object.
    pub(super) fn verify_unique_cached_alias(
        &self,
        requested: &str,
        cached: &str,
    ) -> io::Result<()> {
        if self.case_sensitive_paths || requested == cached {
            return Ok(());
        }
        if !self.paths_equal(requested, cached) {
            return Err(invalid_data("cache identity does not match mounted path"));
        }
        let root = self.projector.root().as_str();
        let cached_parts = relative_components(root, cached)?;
        let requested_parts = relative_components(root, requested)?;
        if cached_parts.len() != requested_parts.len() {
            return Err(invalid_data(
                "case-aliasing mount paths have different depth",
            ));
        }
        let mut parent = root.to_string();
        for (_cached_name, requested_name) in cached_parts.iter().zip(requested_parts) {
            let requested_key = self.name_key(requested_name);
            let matches = self
                .backend
                .list_dir(&parent)?
                .into_iter()
                .filter(|metadata| self.name_key(&metadata.name) == requested_key)
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Err(invalid_data(
                    "backend path is not unique under mount case semantics",
                ));
            }
            parent = join(&parent, &matches[0].name);
        }
        Ok(())
    }
}

fn relative_components<'a>(root: &str, path: &'a str) -> io::Result<Vec<&'a str>> {
    path.strip_prefix(root)
        .filter(|suffix| root == "/" || suffix.starts_with('/'))
        .map(|suffix| {
            suffix
                .trim_start_matches('/')
                .split('/')
                .filter(|component| !component.is_empty())
                .collect()
        })
        .ok_or_else(|| invalid_data("case-aliasing path escaped the mount root"))
}

fn join(parent: &str, child: &str) -> String {
    if parent == "/" {
        format!("/{child}")
    } else {
        format!("{}/{child}", parent.trim_end_matches('/'))
    }
}
