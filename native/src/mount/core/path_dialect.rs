//! Name rules of the filesystem frontend that serves a drive (DRV-M1).
//!
//! `Windows` is the rule set of every release before this one and stays
//! exactly the rules of `path.rs`: separators `\` and `/`, no reserved
//! characters, no DOS device names, no trailing dot or space, at most 255
//! UTF-16 units per name. `Posix` serves the Linux FUSE host and the Android
//! documents provider: callback paths are rooted at `/` and split only at
//! `/`; a name may hold colons, `CON` or a trailing dot, and is rejected only
//! when it is empty, `.` or `..`, holds NUL, `/` or `\`, or exceeds
//! `NAME_MAX` (255 bytes). Backend names with `\` are hidden in both
//! dialects, because several backends treat `\` as a separator.
use serde::{Deserialize, Serialize};
use std::io;

/// `NAME_MAX` of Linux and Android: bytes per name, not characters.
pub(crate) const POSIX_NAME_MAX_BYTES: usize = 255;
/// Bound of a whole POSIX callback path; it only limits memory per request.
const POSIX_PATH_MAX_BYTES: usize = 32_767;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathDialect {
    /// Dokany drives; the default of every stored or transmitted value that
    /// predates the dialect.
    #[default]
    Windows,
    /// Linux FUSE mounts and the Android documents provider.
    Posix,
}

impl PathDialect {
    /// `skip_serializing_if` helper: the Windows default keeps older bytes.
    pub fn is_windows(&self) -> bool {
        matches!(self, Self::Windows)
    }

    /// Whether `component` is one representable name of this dialect.
    pub fn validate_component(self, component: &str) -> io::Result<()> {
        match self {
            Self::Windows => super::path::validate_windows_component(component),
            Self::Posix => validate_posix_component(component),
        }
    }

    /// The callback path a frontend of this dialect uses for a root-relative
    /// forward-slash path (`""` is the root).
    pub fn callback_path(self, relative: &str) -> String {
        match self {
            Self::Windows => format!("\\{}", relative.replace('/', "\\")),
            Self::Posix => format!("/{relative}"),
        }
    }
}

pub(crate) fn validate_posix_component(component: &str) -> io::Result<()> {
    if component.is_empty()
        || matches!(component, "." | "..")
        || component.len() > POSIX_NAME_MAX_BYTES
        || component.contains(['\0', '/', '\\'])
    {
        return Err(invalid(format!(
            "unsafe or unrepresentable POSIX path component: {component:?}"
        )));
    }
    Ok(())
}

/// Splits an untrusted POSIX callback path into validated names. The empty
/// path and `/` are the root (no names).
pub(crate) fn posix_components(callback_path: &str) -> io::Result<Vec<&str>> {
    if callback_path.contains('\0')
        || callback_path.len() > POSIX_PATH_MAX_BYTES
        || (!callback_path.is_empty() && !callback_path.starts_with('/'))
    {
        return Err(invalid("callback path is not a rooted POSIX path"));
    }
    if callback_path.is_empty() || callback_path == "/" {
        return Ok(Vec::new());
    }
    if callback_path.starts_with("//") || callback_path.ends_with('/') {
        return Err(invalid("callback path contains an empty component"));
    }
    callback_path[1..]
        .split('/')
        .map(|component| validate_posix_component(component).map(|()| component))
        .collect()
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}
