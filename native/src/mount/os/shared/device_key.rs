//! The key of a device and the two target types of a Share peer (DRV-M6,
//! contract V4). `PeerOpenTarget` belongs to Share and `PeerMountTarget` to
//! the mount module, so the conversion lives here, outside `core/`.
use crate::mount::{device_key_of, PeerMountTarget};
use crate::share::PeerOpenTarget;

/// The stored target of a mount for a Share target.
pub fn mount_target_of(target: &PeerOpenTarget) -> PeerMountTarget {
    match target {
        PeerOpenTarget::Direct { contact_id } => PeerMountTarget::Direct {
            contact_id: contact_id.clone(),
        },
        PeerOpenTarget::RoomDevice { room_id, device_id } => PeerMountTarget::RoomDevice {
            room_id: room_id.clone(),
            device_id: device_id.clone(),
        },
    }
}

/// The Share target a stored mount target opens.
pub fn open_target_of(target: &PeerMountTarget) -> PeerOpenTarget {
    match target {
        PeerMountTarget::Direct { contact_id } => PeerOpenTarget::Direct {
            contact_id: contact_id.clone(),
        },
        PeerMountTarget::RoomDevice { room_id, device_id } => PeerOpenTarget::RoomDevice {
            room_id: room_id.clone(),
            device_id: device_id.clone(),
        },
    }
}

/// `direct~<contact_id>` or `room~<room_id>~<device_id>`, every byte outside
/// `[A-Za-z0-9._-]` as `%XX` in capital hex: the key of the tiles of se
/// Desktop and of the device tree.
pub fn device_key(target: &PeerOpenTarget) -> String {
    device_key_of(&mount_target_of(target))
}
