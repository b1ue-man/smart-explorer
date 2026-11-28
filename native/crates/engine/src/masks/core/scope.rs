//! Protected paths of a list in se target notation: absolute local paths,
//! `@label:/path`, saved `sftp://`/`ftp://`/`ftps://`/`webdav://` endpoints,
//! `share://direct/…`, `share://room/…`, `gdrive:///…` and `vault://…`
//! (the grammar of the MCP constant `TARGET`). Each scope keeps its backend
//! identity: equal paths on different remotes are different scopes.
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::model::MaskError;

/// Kind of a protected path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ScopeKind {
    /// An absolute local path (drive, UNC or root path), lexically cleaned.
    Local(PathBuf),
    /// A remote location: the target prefix (`@label:`, `sftp://user@host:22`,
    /// `share://direct/<id>`, …) and the path below it.
    Remote { prefix: String, path: String },
}

/// One protected path as stored.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Scope(String);

const REMOTE_SCHEMES: &[&str] = &["sftp://", "ftp://", "ftps://", "webdav://"];

impl Scope {
    /// Validates and normalises a target. Trailing separators are dropped
    /// (except at a root), `.` components removed; `..` is refused because
    /// a protected path must name its location without resolution.
    pub fn parse(text: &str) -> Result<Self, MaskError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(MaskError::InvalidScope("leerer Pfad".into()));
        }
        if text.chars().any(char::is_control) {
            return Err(MaskError::InvalidScope("Steuerzeichen im Pfad".into()));
        }
        let kind = classify(text)?;
        Ok(Self(render(&kind)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn kind(&self) -> ScopeKind {
        // Stored scopes went through `parse`; a damaged one stays a remote
        // literal so it still counts as protected and is never dropped.
        classify(&self.0).unwrap_or_else(|_| ScopeKind::Remote {
            prefix: String::new(),
            path: self.0.clone(),
        })
    }

    pub fn local_path(&self) -> Option<PathBuf> {
        match self.kind() {
            ScopeKind::Local(path) => Some(path),
            ScopeKind::Remote { .. } => None,
        }
    }
}

fn classify(text: &str) -> Result<ScopeKind, MaskError> {
    if let Some(rest) = text.strip_prefix('@') {
        let (label, path) = rest
            .split_once(':')
            .ok_or_else(|| MaskError::InvalidScope("@label:/pfad erwartet".into()))?;
        if label.is_empty() {
            return Err(MaskError::InvalidScope("@label ohne Namen".into()));
        }
        return Ok(ScopeKind::Remote {
            prefix: format!("@{label}:"),
            path: clean_remote_path(path)?,
        });
    }
    for scheme in ["share://direct/", "share://room/"] {
        if let Some(rest) = text.strip_prefix(scheme) {
            let parts = if scheme.ends_with("room/") { 2 } else { 1 };
            let mut split = rest.splitn(parts + 1, '/');
            let mut prefix = scheme.to_string();
            for index in 0..parts {
                let part = split.next().unwrap_or_default();
                if part.is_empty() {
                    return Err(MaskError::InvalidScope(format!("{scheme} ohne Gerät/Raum")));
                }
                if index > 0 {
                    prefix.push('/');
                }
                prefix.push_str(part);
            }
            let path = format!("/{}", split.next().unwrap_or_default());
            return Ok(ScopeKind::Remote {
                prefix,
                path: clean_remote_path(&path)?,
            });
        }
    }
    if let Some(rest) = text.strip_prefix("gdrive://") {
        return Ok(ScopeKind::Remote {
            prefix: "gdrive://".into(),
            path: clean_remote_path(if rest.is_empty() { "/" } else { rest })?,
        });
    }
    if let Some(rest) = text.strip_prefix("vault://") {
        let (site, path) = rest.split_once('/').unwrap_or((rest, ""));
        if site.is_empty() {
            return Err(MaskError::InvalidScope("vault:// ohne Tresor".into()));
        }
        return Ok(ScopeKind::Remote {
            prefix: format!("vault://{site}"),
            path: clean_remote_path(&format!("/{path}"))?,
        });
    }
    for scheme in REMOTE_SCHEMES {
        if let Some(rest) = text.strip_prefix(scheme) {
            let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
            if authority.is_empty() {
                return Err(MaskError::InvalidScope(format!("{scheme} ohne Host")));
            }
            return Ok(ScopeKind::Remote {
                prefix: format!("{scheme}{authority}"),
                path: clean_remote_path(&format!("/{path}"))?,
            });
        }
    }
    if text.contains("://") {
        return Err(MaskError::InvalidScope(format!(
            "unbekannte Zielart in „{text}“"
        )));
    }
    if !is_absolute_local(text) {
        return Err(MaskError::InvalidScope(
            "lokale Pfade müssen absolut sein".into(),
        ));
    }
    Ok(ScopeKind::Local(clean_local(text)?))
}

/// Absolute in the notation of either OS: `/…`, `X:\…`, `X:/…`, `\\server\…`.
fn is_absolute_local(text: &str) -> bool {
    let bytes = text.as_bytes();
    text.starts_with('/')
        || text.starts_with("\\\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/'))
}

/// Lexical cleaning on the text, so the stored form keeps the separators
/// the user wrote on every host (`/srv/x`, `C:\Akten`, `\\server\share`).
fn clean_local(text: &str) -> Result<PathBuf, MaskError> {
    let backslash = text.contains('\\') && !text.starts_with('/');
    let separator = if backslash { '\\' } else { '/' };
    let bytes = text.as_bytes();
    let (prefix, rest) = if text.starts_with("\\\\") {
        ("\\\\".to_string(), &text[2..])
    } else if bytes.len() >= 3 && bytes[1] == b':' {
        (format!("{}{separator}", &text[..2]), &text[3..])
    } else {
        (separator.to_string(), &text[1..])
    };
    let mut parts = Vec::new();
    for part in rest.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return Err(MaskError::InvalidScope("„..“ im Schutzpfad".into())),
            other => parts.push(other),
        }
    }
    Ok(PathBuf::from(format!(
        "{prefix}{}",
        parts.join(&separator.to_string())
    )))
}

fn clean_remote_path(path: &str) -> Result<String, MaskError> {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(MaskError::InvalidScope("„..“ im Schutzpfad".into())),
            other => parts.push(other),
        }
    }
    Ok(format!("/{}", parts.join("/")))
}

fn render(kind: &ScopeKind) -> String {
    match kind {
        ScopeKind::Local(path) => path.to_string_lossy().to_string(),
        ScopeKind::Remote { prefix, path } => format!("{prefix}{path}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_task_scopes_keep_every_target_kind_and_refuse_relative_paths() {
        let cases = [
            ("/srv/klienten/", Some("/srv/klienten")),
            ("/srv/./klienten", Some("/srv/klienten")),
            ("@prod:/srv/data/", Some("@prod:/srv/data")),
            ("sftp://u@host:22/home/x", Some("sftp://u@host:22/home/x")),
            ("ftps://host/a/", Some("ftps://host/a")),
            ("webdav://host/dav", Some("webdav://host/dav")),
            ("share://direct/abc/Docs/", Some("share://direct/abc/Docs")),
            (
                "share://room/r1/dev2/Akten",
                Some("share://room/r1/dev2/Akten"),
            ),
            ("gdrive:///Akten", Some("gdrive:///Akten")),
            ("vault://site/inner", Some("vault://site/inner")),
            ("relative/path", None),
            ("/srv/../etc", None),
            ("foo://x/y", None),
            ("", None),
        ];
        for (input, expected) in cases {
            let parsed = Scope::parse(input).ok();
            assert_eq!(parsed.as_ref().map(Scope::as_str), expected, "{input}");
        }
        assert_eq!(
            Scope::parse("/srv/klienten").unwrap().local_path(),
            Some(PathBuf::from("/srv/klienten"))
        );
        assert_eq!(Scope::parse("@prod:/srv").unwrap().local_path(), None);
        assert_ne!(
            Scope::parse("@a:/x").unwrap(),
            Scope::parse("@b:/x").unwrap(),
            "equal paths on different remotes stay different scopes"
        );
    }
}
