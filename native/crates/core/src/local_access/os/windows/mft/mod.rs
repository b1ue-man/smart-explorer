//! MFT direct reader for the size analysis: reads the master file table of
//! a local NTFS volume in large unbuffered reads instead of listing every
//! directory.
//!
//! The raw volume read bypasses every folder ACL. It therefore exists only
//! behind `LocalElevatedScan`, whose constructor admits a scan only for the
//! caller's own local analysis on this thread: the process token is
//! elevated, the thread impersonates nobody (no Share/peer identity), the
//! backup privilege is available, and the path is a drive-letter path on a
//! local fixed or removable NTFS volume. Share, peer and remote scans and
//! the confined `DirectoryHandle` walk never construct it. Every refusal
//! leaves the caller on the ordinary directory walk.
mod volume;

use super::{
    paths::normalize_scan_root,
    privilege::{parallel_scan_allowed, BackupRead},
    read::open_direct,
};
use crate::local_access::mft::{FileRef, MftEntry, MftIndex, MftScan, ROOT_RECORD};
use crate::local_access::protocol::ReadKind;
use std::{fmt, io, marker::PhantomData, path::Path, rc::Rc, sync::atomic::AtomicBool};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE,
};

/// Cancellation and progress (bytes of `$MFT` read, bytes to read).
pub struct MftControl<'a> {
    pub cancel: &'a AtomicBool,
    pub progress: &'a dyn Fn(u64, u64),
}

/// Why the MFT reader does not run; the caller walks directories instead.
#[derive(Debug)]
pub enum MftRefusal {
    /// The thread impersonates another identity (Share/peer request).
    Impersonating,
    NotElevated,
    NoBackupPrivilege,
    /// Not a drive-letter path (UNC, device or unresolvable path).
    NotLocalPath,
    /// Network, optical, RAM or unknown drive.
    NotLocalDrive,
    NotNtfs(String),
    /// The scan root is a link, a file or lies on another volume than its path says.
    UnsuitableRoot,
    Io(io::Error),
}

impl fmt::Display for MftRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Impersonating => f.write_str("Thread arbeitet unter fremder Identität"),
            Self::NotElevated => f.write_str("Prozess läuft nicht mit Administratorrechten"),
            Self::NoBackupPrivilege => f.write_str("Sicherungsrecht nicht verfügbar"),
            Self::NotLocalPath => f.write_str("kein lokaler Laufwerkspfad"),
            Self::NotLocalDrive => f.write_str("kein lokales festes oder Wechsel-Laufwerk"),
            Self::NotNtfs(name) => write!(f, "Dateisystem {name} ist nicht NTFS"),
            Self::UnsuitableRoot => {
                f.write_str("Startordner ist Verknüpfung, Datei oder anderes Volume")
            }
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for MftRefusal {}

/// Process elevated, thread without impersonation, backup privilege usable.
fn authorized() -> Result<(), MftRefusal> {
    if !parallel_scan_allowed() {
        return Err(MftRefusal::Impersonating);
    }
    if !volume::process_elevated().map_err(MftRefusal::Io)? {
        return Err(MftRefusal::NotElevated);
    }
    BackupRead::enable()
        .map(drop)
        .map_err(|_| MftRefusal::NoBackupPrivilege)
}

fn comparable(path: &str) -> String {
    path.strip_prefix(r"\\?\")
        .unwrap_or(path)
        .trim_end_matches('\\')
        .to_ascii_uppercase()
}

/// An admitted MFT scan of one local folder or volume. Not `Send`: the
/// identity checks hold for the thread that made them.
pub struct LocalElevatedScan {
    device: String,
    serial: u32,
    root: FileRef,
    _thread_only: PhantomData<Rc<()>>,
}

impl LocalElevatedScan {
    /// Admit an MFT scan of `path` for the caller's own local analysis, or
    /// say why not. Never call it for a Share, peer or remote request.
    pub fn for_local_analysis(path: &Path) -> Result<Self, MftRefusal> {
        authorized()?;
        let normalized = normalize_scan_root(path);
        let text = normalized.to_string_lossy();
        let drive = text
            .strip_prefix(r"\\?\")
            .map(str::as_bytes)
            .filter(|rest| rest.len() >= 2 && rest[0].is_ascii_alphabetic() && rest[1] == b':');
        let Some(drive) = drive else {
            return Err(MftRefusal::NotLocalPath);
        };
        let drive_root = format!("{}:\\", char::from(drive[0]).to_ascii_uppercase());
        // GetVolumePathNameW finds a volume mounted in a folder; the drive
        // root is the fallback, checked below by the volume serial.
        let mount = volume::volume_path(normalized.as_os_str()).unwrap_or(drive_root);
        if !volume::local_drive(&mount) {
            return Err(MftRefusal::NotLocalDrive);
        }
        let (file_system, serial) = volume::file_system(&mount).map_err(MftRefusal::Io)?;
        if !file_system.eq_ignore_ascii_case("NTFS") {
            return Err(MftRefusal::NotNtfs(file_system));
        }
        let device = volume::device_path(&mount).map_err(MftRefusal::Io)?;
        let root = if comparable(&text) == comparable(&mount) {
            FileRef {
                record: ROOT_RECORD,
                sequence: 0,
            }
        } else {
            let _backup = BackupRead::enable().map_err(|_| MftRefusal::NoBackupPrivilege)?;
            let folder = open_direct(
                &normalized,
                ReadKind::Metadata,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            )
            .map_err(MftRefusal::Io)?;
            let (attributes, folder_serial, reference) =
                volume::identity(&folder).map_err(MftRefusal::Io)?;
            if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
                || attributes & FILE_ATTRIBUTE_DIRECTORY == 0
                || folder_serial != serial
            {
                return Err(MftRefusal::UnsuitableRoot);
            }
            FileRef::from_raw(reference)
        };
        Ok(Self {
            device,
            serial,
            root,
            _thread_only: PhantomData,
        })
    }

    /// The scan root: record 5 for a volume root, else the folder's reference.
    pub fn root(&self) -> FileRef {
        self.root
    }

    /// Read the volume's whole MFT. Re-checks the thread's authority first.
    pub fn read_index(&self, control: &MftControl<'_>) -> io::Result<MftIndex> {
        authorized().map_err(|refusal| match refusal {
            MftRefusal::Io(error) => error,
            other => io::Error::new(io::ErrorKind::PermissionDenied, other.to_string()),
        })?;
        volume::read_index(&self.device, self.serial, control)
    }

    /// Read the MFT and build the size tree of the scan root, named
    /// `root_name`. `keep` applies the analysis exclusions (app trash,
    /// sync engine and staging names, excluded paths); for a volume root
    /// the NTFS system files come as `MftScan::system_files`.
    pub fn scan(
        &self,
        root_name: &str,
        control: &MftControl<'_>,
        keep: &mut dyn FnMut(&MftEntry<'_>) -> bool,
    ) -> io::Result<MftScan> {
        let index = self.read_index(control)?;
        index.build_tree(self.root, root_name, true, control.cancel, keep)
    }
}
