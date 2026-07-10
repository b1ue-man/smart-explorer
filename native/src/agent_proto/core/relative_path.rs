use std::io;
use std::path::{Path, PathBuf};

const MAX_RELATIVE_PATH_BYTES: usize = 32 * 1024;

/// A wire path proven to be a non-empty, portable path relative to a transfer
/// root. Protocol v6 uses `/` as its only separator. A literal backslash is
/// rejected instead of normalized because it is a valid Unix filename byte and
/// would otherwise collide with a genuinely nested wire path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedRelativePath {
    normalized: String,
}

impl ValidatedRelativePath {
    pub fn parse(raw: &str) -> io::Result<Self> {
        if raw.is_empty() {
            return Err(invalid("empty relative path"));
        }
        if raw.len() > MAX_RELATIVE_PATH_BYTES {
            return Err(invalid("relative path is too long"));
        }
        if raw.starts_with('/') || raw.starts_with('\\') {
            return Err(invalid("absolute relative path"));
        }
        if raw.contains('\\') {
            return Err(invalid("relative path contains a backslash"));
        }
        if raw.as_bytes().contains(&0) {
            return Err(invalid("relative path contains NUL"));
        }

        let mut parts = Vec::new();
        for part in raw.split('/') {
            if part.is_empty() || part == "." || part == ".." {
                return Err(invalid("relative path contains an unsafe component"));
            }
            // A drive prefix or NTFS alternate-data-stream colon is unsafe in
            // every component. On Windows, pushing `C:...` can acquire drive
            // semantics even when it was not the first wire component.
            if has_windows_drive_prefix(part) || part.contains(':') {
                return Err(invalid("relative path contains a drive or stream prefix"));
            }
            parts.push(part);
        }

        Ok(Self {
            normalized: parts.join("/"),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.normalized
    }

    pub fn join_local(&self, root: &Path) -> PathBuf {
        let mut path = root.to_path_buf();
        for part in self.normalized.split('/') {
            path.push(part);
        }
        path
    }

    pub(crate) fn depth(&self) -> usize {
        self.normalized.split('/').count()
    }
}

fn has_windows_drive_prefix(component: &str) -> bool {
    let bytes = component.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::ValidatedRelativePath;
    use std::path::Path;

    #[test]
    fn joins_wire_components_without_reinterpreting_them() {
        let path = ValidatedRelativePath::parse("one/two/three.txt").unwrap();
        assert_eq!(path.as_str(), "one/two/three.txt");
        assert_eq!(
            path.join_local(Path::new("root")),
            Path::new("root").join("one").join("two").join("three.txt")
        );
    }

    #[test]
    fn rejects_paths_that_can_escape_the_transfer_root() {
        for unsafe_path in [
            "",
            "/etc/passwd",
            r"\Windows\system.ini",
            "../escape",
            "safe/../../escape",
            "safe/./file",
            "safe//file",
            r"C:\Windows\system.ini",
            "C:relative.txt",
            "safe/C:escape.txt",
            "safe/file.txt:stream",
            r"literal\backslash.txt",
        ] {
            assert!(
                ValidatedRelativePath::parse(unsafe_path).is_err(),
                "accepted unsafe path {unsafe_path:?}"
            );
        }
        assert!(ValidatedRelativePath::parse(&"a".repeat(32 * 1024 + 1)).is_err());
    }
}
