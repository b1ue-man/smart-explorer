//! Location strings (api.md §2): desktop endpoints plus the app-internal
//! `zip://<archive>!/<inner>` and `trash://`. A location is `prefix + path`,
//! where `path` is the backend path and `prefix` names the connection.
use super::error::ApiError;
use crate::share::PeerOpenTarget;

pub(crate) const TRASH_LOCATION: &str = "trash://";
const ZIP_SCHEME: &str = "zip://";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LocKind {
    Local,
    Sftp,
    Ftp,
    Ftps,
    Webdav,
    GDrive,
    Share,
    Zip,
    Trash,
}

impl LocKind {
    /// The `backend` value of `fs.list`.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            LocKind::Local => "local",
            LocKind::Sftp => "sftp",
            LocKind::Ftp => "ftp",
            LocKind::Ftps => "ftps",
            LocKind::Webdav => "webdav",
            LocKind::GDrive => "gdrive",
            LocKind::Share => "share",
            LocKind::Zip => "zip",
            LocKind::Trash => "trash",
        }
    }
}

/// A parsed location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Loc {
    pub kind: LocKind,
    /// Everything before the backend path (empty for local paths).
    pub prefix: String,
    /// Absolute forward-slash backend path, no trailing slash except `/`.
    pub path: String,
}

impl Loc {
    pub(crate) fn parse(location: &str) -> Result<Self, ApiError> {
        // Only leading blanks go: trailing ones belong to the last name
        // ("Bericht " is not "Bericht").
        let location = location.trim_start();
        if location.is_empty() || location.contains('\0') {
            return Err(ApiError::invalid("Leerer oder ungültiger Ort"));
        }
        if location.starts_with('/') {
            return Self::with(LocKind::Local, String::new(), location);
        }
        let Some((scheme, rest)) = location.split_once("://") else {
            return Err(ApiError::invalid(format!("Ungültiger Ort: {location}")));
        };
        match scheme.to_ascii_lowercase().as_str() {
            "trash" => Ok(Self {
                kind: LocKind::Trash,
                prefix: TRASH_LOCATION.to_string(),
                path: "/".to_string(),
            }),
            "zip" => {
                let (archive, inner) = split_zip(rest);
                if !archive.starts_with('/') {
                    return Err(ApiError::invalid(
                        "ZIP-Orte brauchen einen lokalen Archivpfad",
                    ));
                }
                let archive = normalize_path(archive)?;
                Self::with(LocKind::Zip, format!("{ZIP_SCHEME}{archive}!"), inner)
            }
            "gdrive" => Self::with(
                LocKind::GDrive,
                "gdrive://".to_string(),
                &format!("/{}", rest.trim_start_matches('/')),
            ),
            "share" => {
                let (target, path) = PeerOpenTarget::from_endpoint(location)
                    .ok_or_else(|| ApiError::invalid("Ungültige Share-Adresse"))?;
                Self::with(LocKind::Share, target.endpoint_prefix(), &path)
            }
            scheme @ ("sftp" | "ftp" | "ftps" | "webdav") => {
                let kind = match scheme {
                    "sftp" => LocKind::Sftp,
                    "ftp" => LocKind::Ftp,
                    "ftps" => LocKind::Ftps,
                    _ => LocKind::Webdav,
                };
                let (authority, path) = match rest.find('/') {
                    Some(index) => (&rest[..index], &rest[index..]),
                    None => (rest, "/"),
                };
                if authority.is_empty() {
                    return Err(ApiError::invalid("Ungültige Remote-Adresse"));
                }
                Self::with(kind, format!("{scheme}://{authority}"), path)
            }
            other => Err(ApiError::invalid(format!(
                "Nicht unterstütztes Pfadprotokoll: {other}"
            ))),
        }
    }

    fn with(kind: LocKind, prefix: String, path: &str) -> Result<Self, ApiError> {
        Ok(Self {
            kind,
            prefix,
            path: normalize_path(path)?,
        })
    }

    /// The location string.
    pub(crate) fn location(&self) -> String {
        match self.kind {
            LocKind::Trash => TRASH_LOCATION.to_string(),
            _ => format!("{}{}", self.prefix, self.path),
        }
    }

    /// The location of another backend path on the same connection.
    pub(crate) fn at(&self, path: &str) -> String {
        format!("{}{}", self.prefix, path)
    }

    pub(crate) fn child(&self, name: &str) -> String {
        self.at(&join(&self.path, name))
    }

    /// The last path segment (empty at a root).
    pub(crate) fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or("")
    }

    pub(crate) fn is_root(&self) -> bool {
        self.path == "/"
    }

    pub(crate) fn is_local(&self) -> bool {
        self.kind == LocKind::Local
    }

    pub(crate) fn is_app_internal(&self) -> bool {
        matches!(self.kind, LocKind::Zip | LocKind::Trash)
    }

    /// The archive path of a `zip://` location.
    pub(crate) fn zip_archive(&self) -> Option<&str> {
        if self.kind != LocKind::Zip {
            return None;
        }
        self.prefix
            .strip_prefix(ZIP_SCHEME)
            .and_then(|rest| rest.strip_suffix('!'))
    }

    /// The parent location; an archive root leads back to the archive's folder.
    pub(crate) fn parent(&self) -> Option<String> {
        if self.kind == LocKind::Trash {
            return None;
        }
        if self.is_root() {
            return self.zip_archive().and_then(parent_path).map(str::to_string);
        }
        parent_path(&self.path).map(|parent| self.at(parent))
    }

    /// The favourites key in the desktop format (`location_key`).
    pub(crate) fn favorite_key(&self) -> String {
        match self.kind {
            LocKind::Local => crate::connect::location_key(None, &self.path),
            _ => crate::connect::location_key(Some(&self.prefix), &self.path),
        }
    }
}

/// `true` for `zip://` and `trash://` locations, which never enter desktop
/// formats (favourites, recent, sync jobs, share exports).
pub(crate) fn is_app_internal(location: &str) -> bool {
    let lower = location.trim_start().to_ascii_lowercase();
    lower.starts_with(ZIP_SCHEME) || lower.starts_with(TRASH_LOCATION)
}

/// A `zip://` location for the root of a local archive.
pub(crate) fn zip_location(archive: &str) -> String {
    format!("{ZIP_SCHEME}{archive}!/")
}

/// Splits `<archive>!/<inner>` after the archive: at the first `!/` (or the
/// final `!`) that follows a `.zip` name, so a folder like `Wichtig!` stays
/// part of the archive path; other archive names split at the first `!/`.
fn split_zip(rest: &str) -> (&str, &str) {
    let after_zip = rest
        .match_indices("!/")
        .map(|(index, _)| index)
        .find(|&index| is_zip_name(&rest[..index]));
    if let Some(index) = after_zip {
        return (&rest[..index], &rest[index + 1..]);
    }
    let bare = rest.strip_suffix('!');
    if let Some(archive) = bare.filter(|archive| is_zip_name(archive)) {
        return (archive, "/");
    }
    match rest.find("!/") {
        Some(index) => (&rest[..index], &rest[index + 1..]),
        None => (bare.unwrap_or(rest), "/"),
    }
}

fn is_zip_name(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".zip")
}

pub(crate) fn join(parent: &str, name: &str) -> String {
    if parent.ends_with('/') {
        format!("{parent}{name}")
    } else {
        format!("{parent}/{name}")
    }
}

/// The parent of an absolute path; `None` for `/`.
pub(crate) fn parent_path(path: &str) -> Option<&str> {
    if path == "/" || path.is_empty() {
        return None;
    }
    match path.rfind('/') {
        Some(0) => Some("/"),
        Some(index) => Some(&path[..index]),
        None => None,
    }
}

/// Absolute, no empty/`.`/`..` segments, no trailing slash. Backslashes are
/// literal name characters (Android and Unix remotes allow them).
pub(crate) fn normalize_path(path: &str) -> Result<String, ApiError> {
    let mut parts = Vec::new();
    for part in path.split('/').filter(|part| !part.is_empty()) {
        if part == "." || part == ".." {
            return Err(ApiError::invalid(format!("Ungültiger Pfad: {path}")));
        }
        parts.push(part);
    }
    Ok(format!("/{}", parts.join("/")))
}

/// True when `path` equals `root` or lies below it.
pub(crate) fn is_same_or_below(path: &str, root: &str) -> bool {
    let root = root.trim_end_matches('/');
    path == root
        || root.is_empty()
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// A name the user typed: one non-empty segment without separators.
pub(crate) fn validate_name(name: &str) -> Result<&str, ApiError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ApiError::invalid("Name ist leer"));
    }
    if trimmed == "." || trimmed == ".." || trimmed.contains(['/', '\\', '\0']) {
        return Err(ApiError::invalid("Name darf keine Pfadtrenner enthalten"));
    }
    if trimmed.len() > 255 {
        return Err(ApiError::invalid("Name ist zu lang"));
    }
    Ok(trimmed)
}
