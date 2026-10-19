//! The backend behind a drive's source, resolved in the worker.
use crate::mount::{MountConfig, MountSource, PeerMountTarget};
use crate::vfs::BackendHandle;

use super::ipc_host::ShareHost;

const GDRIVE_ACCOUNT: &str = "cloud:gdrive";

pub(super) fn resolve(config: &MountConfig, host: &ShareHost) -> Result<BackendHandle, String> {
    match &config.source {
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
            if connection.protocol == crate::creds::Protocol::Smb {
                // SMB is browsed, copied and synced, but never mounted as a
                // drive, whatever its staged-write guarantees report.
                return Err("SMB lässt sich nicht als Laufwerk einbinden".into());
            }
            crate::connect::open_saved_at_for_mount(connection, root.as_str(), config.root_security)
                .map(|(backend, _)| backend)
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
        // DRV-M6: the device tree reaches its devices through the host and
        // starts without a backend (`devices_mount.rs`); never asked for.
        MountSource::Devices => Err("Der Gerätebaum hat kein eigenes Backend".into()),
        // DRV-M7 (contract V2, S2-V1): the cleartext view of the unlocked
        // site in this worker; a locked site keeps its code, so the start
        // path defers the drive until the unlock (`mount_termination.rs`).
        MountSource::Vault { site_id } => crate::vault::vault_drive_bridge()
            .open_backend(site_id)
            .map_err(|error| match error.code() {
                Some(code) => format!("{code}: {error}"),
                None => error.to_string(),
            }),
    }
}
