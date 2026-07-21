use crate::mount::{MountSource, PeerMountTarget};
use crate::vfs::BackendHandle;

use super::ipc_host::ShareHost;

const GDRIVE_ACCOUNT: &str = "cloud:gdrive";

pub(super) fn resolve(source: &MountSource, host: &ShareHost) -> Result<BackendHandle, String> {
    match source {
        MountSource::SavedRemote { account, root } => {
            let connections = crate::creds::load_connections_checked()
                .map_err(|error| format!("Gespeicherte Verbindungen lesen: {error}"))?;
            let connection = connections
                .iter()
                .find(|connection| connection.account() == *account)
                .ok_or_else(|| {
                    "Die fuer das Laufwerk gespeicherte Verbindung ist nicht mehr vorhanden"
                        .to_string()
                })?;
            crate::connect::open_saved_at(connection, root.as_str()).map(|(backend, _)| backend)
        }
        MountSource::GoogleDrive { account, root } => {
            if account != GDRIVE_ACCOUNT {
                return Err("Das ausgewaehlte Google-Drive-Konto ist nicht verfuegbar".into());
            }
            crate::connect::open_gdrive(root.as_str()).map(|(backend, _)| backend)
        }
        MountSource::Peer { target, .. } => {
            let target = match target {
                PeerMountTarget::Direct { contact_id } => crate::share::PeerOpenTarget::Direct {
                    contact_id: contact_id.clone(),
                },
                PeerMountTarget::RoomDevice { room_id, device_id } => {
                    crate::share::PeerOpenTarget::RoomDevice {
                        room_id: room_id.clone(),
                        device_id: device_id.clone(),
                    }
                }
            };
            // Deliberately use the daemon-owned ShareHost directly. Going through
            // open_share_backend would create a loopback IPC proxy back into this
            // same process and would split transport fallback state in two.
            host.open_share(target).map(|(_, backend, _)| backend)
        }
    }
}
