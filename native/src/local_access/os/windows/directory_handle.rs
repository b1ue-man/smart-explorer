//! Read pins keep the resolved root and every entered directory alive.
//! Write sharing permits namespace operations; denied delete sharing keeps
//! their pinned paths stable. No per-directory canonicalization is needed.
use std::{
    ffi::OsStr,
    fs::File,
    io,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use std::os::windows::fs::OpenOptionsExt;
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE,
};

use crate::local_access::protocol::ReadKind;
use crate::local_access::{LocalEntry, NotRegular};

#[path = "quarantine.rs"]
mod quarantine;
pub(crate) use quarantine::QuarantinedChild;
#[path = "create.rs"]
mod create;
pub(crate) use create::secure_private_handle;
#[path = "directory_identity.rs"]
mod identity;
#[path = "private_access.rs"]
mod private_access;
#[path = "private_ancestors.rs"]
mod private_ancestors;
#[path = "remove.rs"]
mod remove;

struct PinnedDirectory {
    file: File,
    path: PathBuf,
    _parent: Option<Arc<PinnedDirectory>>,
    _ancestors: Vec<File>,
    /// Present only for a local caller that may use an existing consent.
    consented_path: Option<PathBuf>,
}

impl Drop for PinnedDirectory {
    fn drop(&mut self) {
        // Long trees must not recursively drop an Arc chain on a worker's
        // stack. Shared parents remain owned by their other live handles.
        let mut parent = self._parent.take();
        while let Some(held) = parent {
            match Arc::try_unwrap(held) {
                Ok(mut directory) => parent = directory._parent.take(),
                Err(_) => break,
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct DirectoryHandle(Arc<PinnedDirectory>);

impl DirectoryHandle {
    /// Metadata of the pinned directory itself, without reopening its path.
    pub(crate) fn metadata(&self) -> io::Result<std::fs::Metadata> {
        self.0.file.metadata()
    }

    /// This read pin is synchronous. A path reopen cannot manufacture an
    /// equivalent overlapped watch capability, so callers retain polling.
    pub(crate) fn watch_path(&self) -> Option<PathBuf> {
        None
    }

    /// Only acquiring the chosen root resolves links. All its resulting
    /// physical ancestors remain pinned while this tree is being traversed.
    pub(crate) fn open_root(path: &Path) -> io::Result<Self> {
        let path = super::normalize_scan_root(path);
        // Pin the chosen object before resolving its physical path. A root
        // alias changed during canonicalization cannot substitute another root.
        let selected = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&path)?;
        let resolved = std::fs::canonicalize(&path)?;
        let mut current = PathBuf::new();
        let mut ancestors = Vec::new();
        for component in resolved.components() {
            current.push(component.as_os_str());
            if matches!(component, Component::Prefix(_)) || current == resolved {
                continue;
            }
            let file = super::read::open_direct(
                &current,
                ReadKind::Metadata,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
            )?;
            validate_directory(&file)?;
            ancestors.push(file);
        }
        let file = super::read::open_direct(
            &resolved,
            ReadKind::Directory,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
        )?;
        validate_directory(&file)?;
        if !identity::same_object(&selected, &file)? {
            return Err(io::Error::other(
                "chosen scan root changed while being pinned",
            ));
        }
        Ok(Self(Arc::new(PinnedDirectory {
            file,
            path: resolved,
            _parent: None,
            _ancestors: ancestors,
            consented_path: None,
        })))
    }

    /// Local GUI analysis may use its existing read grant. This never asks
    /// for consent and never replaces an explicit impersonation identity.
    /// Foreign host requests must use `open_root` instead.
    pub(crate) fn open_root_consented(path: &Path) -> io::Result<Self> {
        let logical = super::normalize_scan_root(path);
        let result = Self::open_root(&logical);
        let mut opened = match result {
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                let backed = match super::privilege::BackupRead::enable() {
                    Ok(_backup) => Self::open_root(&logical),
                    Err(_) => Err(error),
                };
                match backed {
                    Err(error)
                        if error.kind() == io::ErrorKind::PermissionDenied
                            && super::privilege::parallel_scan_allowed() =>
                    {
                        let pins =
                            super::broker::pin_granted(&logical, true).unwrap_or(Err(error))?;
                        Self::from_root_pins(pins, logical.clone())?
                    }
                    result => result?,
                }
            }
            result => result?,
        };
        Arc::get_mut(&mut opened.0)
            .ok_or_else(|| io::Error::other("Root-Pin wurde vor der Freigabe geteilt"))?
            .consented_path = Some(logical);
        Ok(opened)
    }

    pub(crate) fn open_child(&self, name: &OsStr) -> io::Result<Self> {
        validate_name(name)?;
        let path = self.0.path.join(name);
        let consented_path = self.0.consented_path.as_ref().map(|path| path.join(name));
        let file = match self.open_kind(&path, consented_path.as_deref(), ReadKind::PinChild) {
            Ok(file) => file,
            Err(error)
                if error.kind() == io::ErrorKind::PermissionDenied
                    && consented_path.is_some()
                    && super::privilege::parallel_scan_allowed() =>
            {
                let logical = consented_path.as_deref().ok_or(error)?;
                let pins = super::broker::pin_granted(logical, false)
                    .ok_or_else(|| io::Error::from(io::ErrorKind::PermissionDenied))??;
                if pins.path != path {
                    return Err(io::Error::other("Lesehelfer wechselte den gepinnten Pfad"));
                }
                return Self::from_pins(pins, Some(self.0.clone()), logical.to_path_buf());
            }
            Err(error) => return Err(error),
        };
        validate_directory(&file)?;
        Ok(Self(Arc::new(PinnedDirectory {
            file,
            path,
            _parent: Some(self.0.clone()),
            _ancestors: Vec::new(),
            consented_path,
        })))
    }

    /// The existing batch reader, including its ordinary-provider fallback,
    /// uses a fresh enumeration handle while pinned ancestors prevent any
    /// redirect of the path that fallback needs.
    pub(crate) fn read_directory(&self) -> io::Result<DirectoryEntries> {
        let file = self.open_kind(
            &self.0.path,
            self.0.consented_path.as_deref(),
            ReadKind::Directory,
        )?;
        Ok(DirectoryEntries {
            inner: super::directory::read_directory_handle(
                &self.0.path,
                file,
                self.0.consented_path.is_some(),
            )?,
            _guard: self.clone(),
        })
    }

    pub(crate) fn open_regular_child(&self, name: &OsStr) -> io::Result<File> {
        validate_name(name)?;
        let file = self.open_entry(name, ReadKind::Metadata)?;
        super::regular::validate_file(&file, true)?;
        let file = self.open_entry(name, ReadKind::File)?;
        super::regular::validate_file(&file, true)?;
        Ok(file)
    }

    pub(super) fn open_entry(&self, name: &OsStr, kind: ReadKind) -> io::Result<File> {
        validate_name(name)?;
        let logical = self.0.consented_path.as_ref().map(|path| path.join(name));
        self.open_kind(&self.0.path.join(name), logical.as_deref(), kind)
    }

    fn open_kind(&self, path: &Path, logical: Option<&Path>, kind: ReadKind) -> io::Result<File> {
        let sharing = if matches!(kind, ReadKind::PinChild | ReadKind::PinRoot) {
            FILE_SHARE_READ | FILE_SHARE_WRITE
        } else {
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE
        };
        let attempt = || super::read::open_direct(path, kind, sharing);
        let result = attempt();
        if !matches!(&result, Err(error) if error.kind() == io::ErrorKind::PermissionDenied)
            || logical.is_none()
        {
            return result;
        }
        let result = match super::privilege::BackupRead::enable() {
            Ok(_backup) => attempt(),
            Err(_) => result,
        };
        if matches!(&result, Err(error) if error.kind() == io::ErrorKind::PermissionDenied)
            && !matches!(kind, ReadKind::PinChild | ReadKind::PinRoot)
            && super::privilege::parallel_scan_allowed()
        {
            if let Some(logical) = logical {
                return super::broker::open_granted(logical, kind).unwrap_or(result);
            }
        }
        result
    }

    fn from_pins(
        mut pins: super::broker::DirectoryPins,
        parent: Option<Arc<PinnedDirectory>>,
        logical: PathBuf,
    ) -> io::Result<Self> {
        for file in &pins.files {
            validate_directory(file)?;
        }
        let file = pins
            .files
            .pop()
            .ok_or_else(|| io::Error::other("Fehlender Ordner-Pin"))?;
        if parent.is_some() && !pins.files.is_empty() {
            return Err(io::Error::other("Ungültige Child-Pins"));
        }
        Ok(Self(Arc::new(PinnedDirectory {
            file,
            path: pins.path,
            _parent: parent,
            _ancestors: pins.files,
            consented_path: Some(logical),
        })))
    }

    pub(super) fn from_root_pins(
        pins: super::broker::DirectoryPins,
        logical: PathBuf,
    ) -> io::Result<Self> {
        let expected = pins
            .path
            .components()
            .filter(|component| !matches!(component, Component::Prefix(_)))
            .count();
        if pins.files.len() != expected {
            return Err(io::Error::other(
                "Lesehelfer lieferte unvollständige Root-Pins",
            ));
        }
        Self::from_pins(pins, None, logical)
    }

    pub(super) fn path(&self) -> &Path {
        &self.0.path
    }

    pub(super) fn file(&self) -> &File {
        &self.0.file
    }

    pub(super) fn pin_files(&self) -> Vec<&File> {
        let mut lineage = Vec::new();
        let mut current = &*self.0;
        loop {
            lineage.push(current);
            let Some(parent) = &current._parent else {
                break;
            };
            current = parent;
        }
        let mut files = Vec::new();
        files.extend(&current._ancestors);
        files.extend(lineage.into_iter().rev().map(|directory| &directory.file));
        files
    }
}

fn validate_name(name: &OsStr) -> io::Result<()> {
    if crate::types::win32_name_issue(&name.to_string_lossy())
        == Some(crate::types::Win32NameIssue::InvalidCharacter)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "entry name contains a Win32 path delimiter",
        ));
    }
    let mut components = Path::new(name).components();
    if matches!(components.next(), Some(Component::Normal(component)) if component == name)
        && components.next().is_none()
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "entry name is not one child component",
        ))
    }
}

fn validate_directory(file: &File) -> io::Result<()> {
    let class = super::directory::classify_open_file(file)?;
    if class.link_like {
        return Err(NotRegular::Link.error());
    }
    if class.special || !file.metadata()?.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            "scan entry is not a regular directory",
        ));
    }
    Ok(())
}

pub(crate) struct DirectoryEntries {
    inner: super::directory::Directory<'static>,
    _guard: DirectoryHandle,
}

impl Iterator for DirectoryEntries {
    type Item = io::Result<LocalEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}
