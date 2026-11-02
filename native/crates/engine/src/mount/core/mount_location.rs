//! Where a drive appears when it is not a Windows drive letter (DRV-M3,
//! strand S2 C2 and C4 of plan 2026-10-17): a folder the user chose.
//!
//! `MountConfig::location` is optional. Absent means the default of the
//! platform: the drive letter of `DriveSelection` under Windows, a folder
//! below `~/SmartExplorer` under Linux (DRV-M4). The field is written only
//! when it is set, so every drive without it keeps the bytes of v0.5.182
//! and an older version ignores it. Only the form of the path is checked
//! here; owner, emptiness and mode of the folder are checked by the host
//! platform right before it mounts (DRV-M4).
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Component, Path, PathBuf};

/// Longest stored folder path in bytes (`PATH_MAX` of Linux).
const MAX_LOCATION_BYTES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MountLocation {
    /// An absolute folder; the root of the drive appears there.
    Directory { path: PathBuf },
}

impl MountLocation {
    /// A checked folder location.
    pub fn directory(path: impl Into<PathBuf>) -> io::Result<Self> {
        let location = Self::Directory { path: path.into() };
        location.validate()?;
        Ok(location)
    }

    pub fn path(&self) -> &Path {
        match self {
            Self::Directory { path } => path,
        }
    }

    /// The path is Unicode, absolute, at most 4096 bytes, free of control
    /// characters and of `.`/`..` segments, and not a filesystem root.
    pub fn validate(&self) -> io::Result<()> {
        let path = self.path();
        let text = path
            .to_str()
            .ok_or_else(|| invalid("drive location must be valid Unicode"))?;
        if text.is_empty()
            || text.len() > MAX_LOCATION_BYTES
            || text.chars().any(|character| character.is_control())
        {
            return Err(invalid(
                "drive location is empty, too long or holds a control character",
            ));
        }
        if !path.is_absolute() {
            return Err(invalid("drive location must be an absolute folder path"));
        }
        let dotted = text
            .split(['/', '\\'])
            .any(|segment| matches!(segment, "." | ".."));
        if dotted
            || path
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        {
            return Err(invalid("drive location may not contain '.' or '..'"));
        }
        if path.file_name().is_none() {
            return Err(invalid("drive location may not be a filesystem root"));
        }
        Ok(())
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
