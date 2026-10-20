//! Document IDs of the Android documents provider (DRV-M8, strand S2 C3).
//!
//! A root is `devices`, `conn:<account as lowercase hex>`, `gdrive` or
//! `vault:<site id>`; a document is its root ID, then `/` and the path
//! below the root (`devices/<device key>/Dokumente/a.txt`). The device key
//! (`mount::device_key`) does not change when a device is renamed, so
//! granted URIs stay valid. Every check here is pure: `is_child` answers
//! without I/O, as DocumentsUI asks it for every grant.
use std::fmt;

use crate::mount::PathDialect;

pub(super) const DEVICES: &str = "devices";
pub(super) const GDRIVE: &str = "gdrive";
const CONN_PREFIX: &str = "conn:";
const VAULT_PREFIX: &str = "vault:";

/// One root of the provider.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum RootId {
    Devices,
    /// A saved connection by its account key (`SavedConnection::account`).
    Conn(String),
    GDrive,
    /// An unlocked vault by its site ID.
    Vault(String),
}

impl RootId {
    pub(super) fn parse(text: &str) -> Option<Self> {
        if text == DEVICES {
            return Some(Self::Devices);
        }
        if text == GDRIVE {
            return Some(Self::GDrive);
        }
        if let Some(hex) = text.strip_prefix(CONN_PREFIX) {
            return unhex(hex).map(Self::Conn);
        }
        let site = text.strip_prefix(VAULT_PREFIX)?;
        crate::vault::is_site_id(site).then(|| Self::Vault(site.to_string()))
    }

    /// The settings key: the account, `gdrive`, the site ID or `devices`.
    pub(super) fn setting_key(&self) -> &str {
        match self {
            Self::Devices => DEVICES,
            Self::GDrive => GDRIVE,
            Self::Conn(account) => account,
            Self::Vault(site) => site,
        }
    }
}

impl fmt::Display for RootId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Devices => formatter.write_str(DEVICES),
            Self::GDrive => formatter.write_str(GDRIVE),
            Self::Conn(account) => write!(formatter, "{CONN_PREFIX}{}", hex(account)),
            Self::Vault(site) => write!(formatter, "{VAULT_PREFIX}{site}"),
        }
    }
}

/// A document: its root and the `/`-separated path below the root (empty
/// for the root folder itself).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct DocId {
    pub(super) root: RootId,
    pub(super) path: String,
}

impl DocId {
    pub(super) fn root_of(root: RootId) -> Self {
        Self {
            root,
            path: String::new(),
        }
    }

    /// Parses and checks every name (Posix rules: not empty, not `.` or
    /// `..`, no `/`, `\` or NUL).
    pub(super) fn parse(text: &str) -> Option<Self> {
        let (root, path) = text.split_once('/').unwrap_or((text, ""));
        let root = RootId::parse(root)?;
        if !path.is_empty()
            && !path
                .split('/')
                .all(|name| PathDialect::Posix.validate_component(name).is_ok())
        {
            return None;
        }
        if root == RootId::Devices && !path.is_empty() && !valid_key(first(path)) {
            return None;
        }
        Some(Self {
            root,
            path: path.to_string(),
        })
    }

    pub(super) fn is_root(&self) -> bool {
        self.path.is_empty()
    }

    /// The document `name` directly inside this one.
    pub(super) fn child(&self, name: &str) -> Option<Self> {
        PathDialect::Posix.validate_component(name).ok()?;
        if self.root == RootId::Devices && self.is_root() && !valid_key(name) {
            return None;
        }
        let path = if self.path.is_empty() {
            name.to_string()
        } else {
            format!("{}/{name}", self.path)
        };
        Some(Self {
            root: self.root.clone(),
            path,
        })
    }

    /// The folder this document is in; `None` for the root.
    pub(super) fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        let path = self.path.rsplit_once('/').map_or("", |(parent, _)| parent);
        Some(Self {
            root: self.root.clone(),
            path: path.to_string(),
        })
    }

    /// The last name; the root ID for the root.
    pub(super) fn name(&self) -> String {
        match self.path.rsplit_once('/') {
            Some((_, name)) => name.to_string(),
            None if self.path.is_empty() => self.root.to_string(),
            None => self.path.clone(),
        }
    }

    /// For the device tree: the device key and the engine path below the
    /// device (`/` for the device folder). `None` for the tree root.
    pub(super) fn device_part(&self) -> Option<(&str, String)> {
        if self.root != RootId::Devices || self.path.is_empty() {
            return None;
        }
        let (key, rest) = self.path.split_once('/').unwrap_or((&self.path, ""));
        Some((key, format!("/{rest}")))
    }

    /// The engine path of a root with one engine (`/` for the root).
    pub(super) fn engine_path(&self) -> String {
        format!("/{}", self.path)
    }
}

impl fmt::Display for DocId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            write!(formatter, "{}", self.root)
        } else {
            write!(formatter, "{}/{}", self.root, self.path)
        }
    }
}

/// `isChildDocument`: whether `child` lies inside `parent` (or is it), by
/// the IDs alone. Both must be valid IDs of the same root.
pub(super) fn is_child(parent: &str, child: &str) -> bool {
    let (Some(parent), Some(child)) = (DocId::parse(parent), DocId::parse(child)) else {
        return false;
    };
    if parent.root != child.root {
        return false;
    }
    parent.path.is_empty()
        || child.path == parent.path
        || child
            .path
            .strip_prefix(&parent.path)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn first(path: &str) -> &str {
    path.split('/').next().unwrap_or("")
}

/// A device key of `mount::device_key`: `[A-Za-z0-9._~%-]` only.
fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'~' | b'%')
        })
}

fn hex(text: &str) -> String {
    text.bytes().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(hex: &str) -> Option<String> {
    if hex.is_empty() || hex.len() % 2 != 0 {
        return None;
    }
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|at| {
            let pair = hex.get(at..at + 2)?;
            if pair.bytes().any(|byte| byte.is_ascii_uppercase()) {
                return None;
            }
            u8::from_str_radix(pair, 16).ok()
        })
        .collect::<Option<Vec<u8>>>()?;
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SITE: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn drive_task_docs_ids_round_trip_every_root() {
        let roots = [
            RootId::Devices,
            RootId::GDrive,
            RootId::Conn("sftp://anna@host:22/home/anna".into()),
            RootId::Vault(SITE.into()),
        ];
        for root in roots {
            let text = root.to_string();
            assert_eq!(RootId::parse(&text), Some(root.clone()), "{text}");
            let doc = DocId::root_of(root).child("a b.txt").unwrap();
            assert_eq!(DocId::parse(&doc.to_string()), Some(doc.clone()));
        }
        assert_eq!(
            RootId::Conn("sftp://a@h:22/".into()).to_string(),
            "conn:736674703a2f2f6140683a32322f"
        );
    }

    #[test]
    fn drive_task_docs_ids_reject_escapes() {
        for bad in [
            "devices/../x",
            "gdrive/a//b",
            "gdrive/a/./b",
            "gdrive/a\\b",
            "vault:0123/a",
            "conn:ABCD/a",
            "conn:6/a",
            "other/a",
            "devices/bad key/a",
        ] {
            assert_eq!(DocId::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn drive_task_docs_is_child_without_io() {
        let vault = format!("vault:{SITE}");
        assert!(is_child("gdrive", "gdrive/a/b"));
        assert!(is_child("gdrive/a", "gdrive/a/b"));
        assert!(is_child("gdrive/a", "gdrive/a"));
        assert!(!is_child("gdrive/a", "gdrive/ab"));
        assert!(!is_child("gdrive", &format!("{vault}/a")));
        assert!(!is_child("gdrive/a", "gdrive/../a/b"));
        assert!(is_child(&vault, &format!("{vault}/x")));
    }

    #[test]
    fn drive_task_docs_device_ids_keep_the_key() {
        let doc = DocId::parse("devices/direct~c1/Dokumente/a.txt").unwrap();
        assert_eq!(
            doc.device_part(),
            Some(("direct~c1", "/Dokumente/a.txt".into()))
        );
        let device = DocId::parse("devices/direct~c1").unwrap();
        assert_eq!(device.device_part(), Some(("direct~c1", "/".into())));
        assert_eq!(DocId::parse("devices").unwrap().device_part(), None);
        assert_eq!(
            doc.parent().unwrap().to_string(),
            "devices/direct~c1/Dokumente"
        );
        assert_eq!(doc.name(), "a.txt");
    }
}
