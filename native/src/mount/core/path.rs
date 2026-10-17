use super::path_dialect::{posix_components, PathDialect};
use super::types::BackendRoot;
use std::io;

const MAX_COMPONENT_UTF16: usize = 255;
const MAX_PATH_UTF16: usize = 32_767;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectedPath {
    relative: String,
    backend: String,
}

impl ProjectedPath {
    pub fn relative(&self) -> &str {
        &self.relative
    }

    pub fn backend(&self) -> &str {
        &self.backend
    }
}

#[derive(Clone, Debug)]
pub struct PathProjector {
    root: BackendRoot,
    dialect: PathDialect,
}

impl PathProjector {
    /// Windows callback paths, the rules of every release before DRV-M1.
    pub fn new(root: BackendRoot) -> Self {
        Self::with_dialect(root, PathDialect::Windows)
    }

    pub fn with_dialect(root: BackendRoot, dialect: PathDialect) -> Self {
        Self { root, dialect }
    }

    pub fn root(&self) -> &BackendRoot {
        &self.root
    }

    pub fn dialect(&self) -> PathDialect {
        self.dialect
    }

    /// Projects an untrusted filesystem callback path of this projector's
    /// dialect below the exact configured backend root. No aliasing form is
    /// normalized.
    pub fn project(&self, callback_path: &str) -> io::Result<ProjectedPath> {
        let relative = match self.dialect {
            PathDialect::Windows => windows_relative(callback_path)?,
            PathDialect::Posix => posix_components(callback_path)?.join("/"),
        };
        if relative.is_empty() {
            return Ok(ProjectedPath {
                relative,
                backend: self.root.as_str().to_string(),
            });
        }
        let backend = join_backend(self.root.as_str(), &relative);
        if !within_root(self.root.as_str(), &backend) {
            return Err(invalid("projected path escapes its backend root"));
        }
        Ok(ProjectedPath { relative, backend })
    }

    pub(crate) fn ancestor_paths(&self, projected: &ProjectedPath) -> Vec<String> {
        let mut ancestors = Vec::new();
        let mut current = self.root.as_str().trim_end_matches('/').to_string();
        let mut components = projected.relative.split('/').peekable();
        while let Some(component) = components.next() {
            if components.peek().is_none() {
                break;
            }
            current = join_backend(if current.is_empty() { "/" } else { &current }, component);
            ancestors.push(current.clone());
        }
        ancestors
    }
}

/// The forward-slash relative path of a Windows callback path; empty for the
/// root. Unchanged rules of every release before DRV-M1.
fn windows_relative(callback_path: &str) -> io::Result<String> {
    if callback_path.contains('\0')
        || callback_path.encode_utf16().count() > MAX_PATH_UTF16
        || (!callback_path.is_empty()
            && !callback_path.starts_with('\\')
            && !callback_path.starts_with('/'))
    {
        return Err(invalid("callback path is not a rooted Windows path"));
    }
    if callback_path.is_empty() || matches!(callback_path, "\\" | "/") {
        return Ok(String::new());
    }
    if callback_path.starts_with("\\\\")
        || callback_path.starts_with("//")
        || callback_path.ends_with('\\')
        || callback_path.ends_with('/')
    {
        return Err(invalid("callback path contains an empty component"));
    }
    let mut components = Vec::new();
    for component in callback_path[1..].split(['\\', '/']) {
        validate_windows_component(component)?;
        components.push(component);
    }
    Ok(components.join("/"))
}

pub(crate) fn validate_windows_component(component: &str) -> io::Result<()> {
    if component.is_empty()
        || matches!(component, "." | "..")
        || component.ends_with('.')
        || component.ends_with(' ')
        || component.encode_utf16().count() > MAX_COMPONENT_UTF16
        || component.chars().any(|character| {
            character == '\0'
                || character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
        })
        || is_dos_device_name(component)
    {
        return Err(invalid(format!(
            "unsafe or unrepresentable Windows path component: {component:?}"
        )));
    }
    Ok(())
}

fn is_dos_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    let upper = stem.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CLOCK$") {
        return true;
    }
    if upper.len() == 4 {
        let (prefix, suffix) = upper.split_at(3);
        return matches!(prefix, "COM" | "LPT")
            && matches!(suffix.as_bytes().first().copied(), Some(b'1'..=b'9'));
    }
    let mut chars = upper.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    matches!(prefix.as_str(), "COM" | "LPT") && matches!(chars.next(), Some('¹' | '²' | '³'))
}

fn join_backend(root: &str, relative: &str) -> String {
    if relative.is_empty() {
        return root.to_string();
    }
    if root == "/" {
        format!("/{relative}")
    } else {
        format!("{}/{relative}", root.trim_end_matches('/'))
    }
}

fn within_root(root: &str, candidate: &str) -> bool {
    candidate == root
        || (root == "/" && candidate.starts_with('/'))
        || candidate
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}
