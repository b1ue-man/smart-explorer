//! A drive that mounts a remote place (a Share device, a saved connection,
//! Google Drive) is analysed by that place instead of being walked through
//! the mount: a Share device analyses its own disk with its local worker,
//! exactly as "📡 Remote-Ordner" does. The analysis stays the drive path for
//! everything shown and opened; only the walk goes to the remote.
use super::prelude::format_bytes;
use crate::mount::{MountSnapshot, MountSource, MountStatus, PeerMountTarget};

/// Keep host facts when the visible source remains a mounted drive path.
pub(in crate::app) fn host_notes(outcome: &mut crate::analytics::ScanOutcome) {
    if let Some(volume) = outcome.volume.filter(|volume| volume.total_bytes > 0) {
        outcome.notes.push(format!(
            "Speichervolumen der Gegenstelle: {} von {} belegt; {} frei.",
            format_bytes(volume.used_bytes()),
            format_bytes(volume.total_bytes),
            format_bytes(volume.free_bytes)
        ));
    }
    if let Some((figures, tree)) = outcome.platform.as_ref().zip(outcome.tree.as_ref()) {
        let approx = crate::analytics::Approximations::compute(
            tree,
            &figures.place(),
            figures.totals(),
            outcome.status == crate::analytics::ScanStatus::Complete,
        );
        if let Some(view) = crate::analytics::node_view(tree, &[], &approx, 0) {
            for row in view.children {
                outcome.notes.push(format!(
                    "{}: {} (Angaben der Gegenstelle).",
                    row.name,
                    format_bytes(row.size)
                ));
            }
        }
    }
}

/// Where the analysis of a mounted drive path really runs.
pub(in crate::app) struct MountRoute {
    source: MountSource,
    /// The backend path that the drive path names.
    path: String,
    label: String,
}

impl MountRoute {
    /// The route of `root` (`X:/…`) when a mount of the current snapshot
    /// serves drive `X:`; `None` for every other path.
    pub(in crate::app) fn of(mounts: &[MountSnapshot], root: &str) -> Option<Self> {
        let letter = drive_letter(root)?;
        let mount = mounts.iter().find(
            |mount| matches!(mount.status, MountStatus::Mounted { drive } if drive.get() == letter),
        )?;
        let below = root[2..].trim_start_matches('/');
        let base = mount.config.source.root().as_str().trim_end_matches('/');
        let path = match (base.is_empty(), below.is_empty()) {
            (true, true) => "/".to_string(),
            (true, false) => format!("/{below}"),
            (false, true) => base.to_string(),
            (false, false) => format!("{base}/{below}"),
        };
        Some(Self {
            source: mount.config.source.clone(),
            path,
            label: mount.config.label.clone(),
        })
    }

    /// Analyses the remote behind the drive. When it cannot be opened
    /// directly, the drive itself is walked as before, with a note.
    pub(in crate::app) fn scan(
        self,
        local_root: &str,
        progress: &crate::analytics::Progress,
    ) -> crate::analytics::ScanOutcome {
        match self.open() {
            Ok(backend) => crate::analytics::scan_remote(&*backend, &self.path, progress),
            Err(error) => {
                let native = local_root.replace('/', std::path::MAIN_SEPARATOR_STR);
                let mut outcome = crate::analytics::scan(std::path::Path::new(&native), progress);
                outcome.notes.push(format!(
                    "„{}“ war nicht direkt erreichbar ({error}); das Laufwerk wurde Ordner für Ordner gelesen.",
                    self.label
                ));
                outcome
            }
        }
    }

    fn open(&self) -> Result<crate::vfs::BackendHandle, String> {
        let backend = match &self.source {
            MountSource::Peer { target, .. } => {
                let target = match target {
                    PeerMountTarget::Direct { contact_id } => {
                        crate::share::PeerOpenTarget::Direct {
                            contact_id: contact_id.clone(),
                        }
                    }
                    PeerMountTarget::RoomDevice { room_id, device_id } => {
                        crate::share::PeerOpenTarget::RoomDevice {
                            room_id: room_id.clone(),
                            device_id: device_id.clone(),
                        }
                    }
                };
                crate::daemon::open_share_backend(target)?.1
            }
            MountSource::SavedRemote { account, .. } => {
                let connections = crate::creds::load_connections_checked()?;
                let connection = connections
                    .iter()
                    .find(|connection| connection.account() == *account)
                    .ok_or_else(|| "gespeicherte Verbindung fehlt".to_string())?;
                crate::connect::open_saved_at(connection, &self.path)?.0
            }
            MountSource::GoogleDrive { .. } => crate::connect::open_gdrive(&self.path)?.0,
        };
        // A full walk must not fill the browsing cache.
        Ok(crate::vfs::sync_backend(backend))
    }
}

/// `X` of a drive path `X:/…` (any platform; only drives have mounts here).
fn drive_letter(root: &str) -> Option<char> {
    let bytes = root.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        Some(char::from(bytes[0]).to_ascii_uppercase())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mount::{
        BackendRoot, DriveLetter, DriveSelection, MountConfig, MountId, MountMode, MountRecovery,
    };

    fn mounted(letter: char, root: &str) -> MountSnapshot {
        let source = MountSource::Peer {
            target: PeerMountTarget::Direct {
                contact_id: "contact".into(),
            },
            root: BackendRoot::parse(root).expect("root"),
        };
        let id = MountId::new_random().expect("id");
        let config = MountConfig::new(
            id,
            source,
            DriveSelection::Automatic,
            MountMode::ReadOnly,
            "PC",
        )
        .expect("config");
        MountSnapshot {
            config,
            status: MountStatus::Mounted {
                drive: DriveLetter::parse(letter).expect("letter"),
            },
            recovery: MountRecovery::default(),
            recovery_required_compat: false,
        }
    }

    #[test]
    fn review_task_mounted_drive_paths_route_to_the_remote() {
        let mounts = [mounted('Z', "/Daten")];
        let route = MountRoute::of(&mounts, "z:/Fotos/2024").expect("route");
        assert_eq!(route.path, "/Daten/Fotos/2024");
        assert_eq!(route.label, "PC");
        assert_eq!(
            MountRoute::of(&mounts, "Z:/").map(|route| route.path),
            Some("/Daten".into())
        );
        assert!(MountRoute::of(&mounts, "C:/Daten").is_none());
        assert!(MountRoute::of(&mounts, "/home/user").is_none());
        let at_root = [mounted('Y', "/")];
        assert_eq!(
            MountRoute::of(&at_root, "Y:/a").map(|route| route.path),
            Some("/a".into())
        );
    }
}
