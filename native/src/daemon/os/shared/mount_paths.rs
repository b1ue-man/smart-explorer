//! Where a location lies on a drive (DRV-M7; contract V2, S2-V8): the open
//! rule of the vault strand (C7.2) opens a vault file through the vault's
//! drive, so other programs read and save it there and its cleartext never
//! lands in a temporary copy.
//!
//! - `mount_path_for(location)`: the path of `location` on a mounted drive
//!   of the running worker (`DriveIpcRequest::MountPathFor`); `Ok(None)`
//!   when no drive serves it, no worker runs, or the name cannot exist on
//!   that drive (a Windows drive cannot show `a:b`).
//! - `ensure_mounted_for(location)`: the same, mounting the vault first when
//!   it has no mounted drive (an existing drive of the vault is retried,
//!   else a new read-write drive with the vault's label is started). Only
//!   `vault://` locations mount on demand.
//!
//! Drives map `vault://<site>/<path>` (any spelling of `<SITE>`; the worker
//! compares site IDs) and `gdrive:///<path>` below the drive's root. The
//! mount path comes from `MountStatus::MountedAt` (Linux folder) or the
//! letter of `Mounted`/`Conflict` (Windows).
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::mount::{
    DriveSelection, MountConfig, MountId, MountMode, MountSnapshot, MountSource, MountStatus, MountRootSecurity,
    PathDialect,
};

use super::super::drive_modules::ipc_protocol_drives::DriveIpcRequest;
use super::super::ipc_protocol::{
    read_response, set_stream_timeout, write_request, IpcRequest, IpcResponse,
};
use super::super::ipc_storage::{read_ipc_addr, read_token};

/// A drive that mounts on demand gets this long.
const MOUNT_WAIT: Duration = Duration::from_secs(120);
const ASK_TIMEOUT: Duration = Duration::from_secs(10);

/// A location a drive can serve.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Located {
    /// `vault://<SITE>/<path>`; `site` as written (ID, prefix or label).
    Vault { site: String, path: String },
    /// `gdrive:///<path>`.
    GoogleDrive { path: String },
}

impl Located {
    /// `None` for every location no drive kind serves.
    pub(crate) fn parse(location: &str) -> Option<Self> {
        let location = location.trim();
        if let Ok(locator) = crate::vault::VaultLocator::parse(location) {
            return Some(Self::Vault {
                site: locator.site,
                path: locator.path,
            });
        }
        let rest = location.strip_prefix("gdrive://")?;
        Some(Self::GoogleDrive {
            path: format!("/{}", rest.trim_start_matches('/')),
        })
    }
}

/// The path below the drive root of `source` that `located` names; `site`
/// is the site ID `located` resolved to.
pub(crate) fn relative_in(source: &MountSource, located: &Located, site: &str) -> Option<String> {
    match (source, located) {
        (MountSource::Vault { site_id }, Located::Vault { path, .. }) if site_id == site => {
            Some(path.trim_start_matches('/').to_string())
        }
        (MountSource::GoogleDrive { root, .. }, Located::GoogleDrive { path }) => {
            below(root.as_str(), path)
        }
        _ => None,
    }
}

fn below(root: &str, path: &str) -> Option<String> {
    let root = root.trim_end_matches('/');
    if root.is_empty() {
        return Some(path.trim_start_matches('/').to_string());
    }
    let rest = path.strip_prefix(root)?;
    match rest {
        "" => Some(String::new()),
        rest if rest.starts_with('/') => Some(rest.trim_start_matches('/').to_string()),
        _ => None,
    }
}

/// The folder (Linux) or letter root (Windows) of a mounted drive.
pub(crate) fn mounted_root(status: &MountStatus) -> Option<(PathBuf, PathDialect)> {
    match status {
        MountStatus::MountedAt { path } => Some((path.clone(), PathDialect::Posix)),
        MountStatus::Mounted { drive } | MountStatus::Conflict { drive, .. } => Some((
            PathBuf::from(format!("{}:\\", drive.get())),
            PathDialect::Windows,
        )),
        _ => None,
    }
}

/// `relative` below `root`; `None` when a name is not one the drive shows.
pub(crate) fn path_on_drive(root: &Path, dialect: PathDialect, relative: &str) -> Option<PathBuf> {
    let mut path = root.to_path_buf();
    for component in relative
        .split('/')
        .filter(|component| !component.is_empty())
    {
        dialect.validate_component(component).ok()?;
        path.push(component);
    }
    Some(path)
}

/// The path of `located` on `snapshot`'s drive, when it is mounted.
pub(crate) fn drive_path(
    snapshot: &MountSnapshot,
    located: &Located,
    site: &str,
) -> Option<PathBuf> {
    let relative = relative_in(&snapshot.config.source, located, site)?;
    let (root, dialect) = mounted_root(&snapshot.status)?;
    path_on_drive(&root, dialect, &relative)
}

/// Worker side of `MountPathFor`: the first mounted drive that serves
/// `location`, at most one.
pub(in crate::daemon) fn mounts_for_location(
    mounts: Vec<MountSnapshot>,
    location: &str,
) -> Result<Vec<MountSnapshot>, String> {
    let Some(located) = Located::parse(location) else {
        return Ok(Vec::new());
    };
    let site = canonical_site(&located)?;
    let mut mounts = mounts;
    mounts.sort_by_key(mount_rank);
    Ok(mounts
        .into_iter()
        .filter(|mount| current_mount_policy(mount, &site))
        .find(|mount| drive_path(mount, &located, &site).is_some())
        .into_iter()
        .collect())
}

/// The site ID of a vault location (empty for other locations).
fn canonical_site(located: &Located) -> Result<String, String> {
    match located {
        Located::Vault { site, .. } => crate::vault::RegistryStore::open_default()
            .resolve(site)
            .map(|record| record.id),
        Located::GoogleDrive { .. } => Ok(String::new()),
    }
}

/// The path of `location` on a mounted drive of the running worker.
pub(crate) fn mount_path_for(location: &str) -> Result<Option<PathBuf>, String> {
    let Some(located) = Located::parse(location) else {
        return Ok(None);
    };
    let site = canonical_site(&located)?;
    let Some(addr) = read_ipc_addr().filter(|addr| addr.ip().is_loopback()) else {
        return Ok(None);
    };
    let Ok(token) = read_token() else {
        return Ok(None);
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(2)) else {
        return Ok(None);
    };
    set_stream_timeout(&stream, Some(ASK_TIMEOUT));
    let request = IpcRequest::Drive {
        token,
        req: DriveIpcRequest::MountPathFor {
            location: location.to_string(),
        },
    };
    write_request(&mut stream, &request)
        .map_err(|error| format!("Laufwerk-IPC senden: {error}"))?;
    match read_response(&mut stream).map_err(|error| format!("Laufwerk-IPC lesen: {error}"))? {
        IpcResponse::Mounts { mounts } => Ok(mounts
            .iter()
            .find_map(|mount| drive_path(mount, &located, &site))),
        IpcResponse::Err { msg, .. } => Err(msg),
        _ => Err("Unerwartete Laufwerk-IPC-Antwort".into()),
    }
}

/// The path of `location` on a drive; a vault without a mounted drive is
/// mounted first.
pub(crate) fn ensure_mounted_for(location: &str) -> Result<PathBuf, String> {
    if let Some(path) = mount_path_for(location)? {
        return Ok(path);
    }
    let Some(located @ Located::Vault { .. }) = Located::parse(location) else {
        return Err(
            "Nur Tresororte (vault://) werden auf Anforderung als Laufwerk eingebunden".into(),
        );
    };
    let site = canonical_site(&located)?;
    let policy = crate::vault::mount_policy(&site)?;
    if !policy.allowed() {
        return Err(format!("{}: Diesem Speicherort für das Einhängen vertrauen", crate::vault::ROOT_TRUST_REQUIRED));
    }
    let label = crate::vault::RegistryStore::open_default()
        .resolve(&site)
        .map(|record| record.display_label().to_string())?;
    let mut candidates = super::list_mounts()?;
    candidates.sort_by_key(mount_rank);
    let waiting = candidates.into_iter().find(|mount| {
        matches!(&mount.config.source, MountSource::Vault { site_id } if *site_id == site)
            && !matches!(mount.status, MountStatus::Mounted { .. } | MountStatus::MountedAt { .. } | MountStatus::Conflict { .. })
            && (!mount.recovery.requires_retention() || matches!(mount.status, MountStatus::Mounting))
            && current_mount_policy(mount, &site)
    });
    // A concurrent opener may already be mounting this exact site. Observe
    // that same operation rather than restarting its host.
    let started = match waiting {
        Some(mount) if matches!(mount.status, MountStatus::Mounting) => mount,
        Some(mount) => super::retry_mount(mount.config.id)?,
        None => {
            let id = MountId::new_random().map_err(|error| error.to_string())?;
            let source = MountSource::Vault {
                site_id: site.clone(),
            };
            let config = MountConfig::new(
                id,
                source,
                DriveSelection::Automatic,
                if policy.read_only { MountMode::ReadOnly } else { MountMode::ReadWrite },
                label.chars().take(128).collect::<String>(),
            )
            .map_err(|error| format!("Tresor-Laufwerk: {error}"))?
            .with_root_security(if policy.enforced { MountRootSecurity::Enforced } else { MountRootSecurity::Trusted });
            super::start_mount(config)?
        }
    };
    let id = started.config.id.clone();
    let deadline = Instant::now() + MOUNT_WAIT;
    let mut current = started;
    while matches!(current.status, MountStatus::Mounting) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(150));
        current = super::list_mounts()?
            .into_iter()
            .find(|mount| mount.config.id == id)
            .ok_or_else(|| "Das Tresor-Laufwerk verschwand beim Einbinden".to_string())?;
    }
    if let Some(path) = mount_path_for(location)? {
        return Ok(path);
    }
    Err(match current.status {
        MountStatus::Failed { detail } => detail,
        MountStatus::RuntimeUnavailable { detail } => format!("{}: {detail}", crate::vault::MOUNT_RUNTIME_MISSING),
        MountStatus::Mounting => "Das Tresor-Laufwerk wurde nicht rechtzeitig eingebunden".into(),
        _ => "Der Ort liegt nicht auf dem Tresor-Laufwerk".into(),
    })
}

/// A healthy mounted drive wins over a stale strict or recovery entry.
fn mount_rank(mount: &MountSnapshot) -> u8 {
    match mount.status {
        MountStatus::Mounted { .. } | MountStatus::MountedAt { .. } => {
            if mount.recovery.requires_retention() { 1 } else { 0 }
        }
        MountStatus::Mounting => 2,
        MountStatus::Conflict { .. } => 5,
        _ if mount.recovery.requires_retention() => 4,
        _ => 3,
    }
}

fn current_mount_policy(mount: &MountSnapshot, site: &str) -> bool {
    if !matches!(mount.config.source, MountSource::Vault { .. }) { return true; }
    if matches!(mount.status, MountStatus::Conflict { .. }) { return false; }
    let Ok(policy) = crate::vault::mount_policy(site) else { return false; };
    policy.allowed() && (!policy.read_only || mount.config.mode == MountMode::ReadOnly)
        && (!matches!(mount.status, MountStatus::Mounted { .. } | MountStatus::MountedAt { .. } | MountStatus::Conflict { .. })
            || crate::vault::mount_binding_matches(mount.config.id.as_str(), &policy.binding))
}

#[cfg(test)]
#[path = "mount_paths_tests.rs"]
mod tests;
