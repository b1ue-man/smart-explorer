//! Types of the device tree „Verbundene Geräte als Ordner“ (DRV-M6): one
//! row per paired device, the link to the devices and the configuration of
//! the tree. The tree itself is `device_tree.rs`.
use std::any::Any;
use std::io;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::cache_policy::MountCachePolicy;
use super::cache_space::CacheSpaceProbe;
use super::metadata_policy::MountMetadataPolicy;
use super::mount_source::PeerMountTarget;
use super::types::{BackendRoot, MountId};
use crate::vfs::BackendHandle;

/// A Direct contact or a member of a room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Direct,
    Room,
}

/// The room a device is listed under.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomRef {
    pub id: String,
    pub name: String,
}

/// One device of the tree. `name` is the display name without the name rules
/// of the folder; `key` is `device_key_of(target)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRow {
    pub key: String,
    pub name: String,
    pub kind: DeviceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room: Option<RoomRef>,
    pub online: bool,
    pub target: PeerMountTarget,
}

/// One shared folder of a device: `name` is its segment below the device
/// folder (`Dokumente` or `Verbindungen/Server`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChildRoot {
    pub root: BackendRoot,
    pub name: String,
}

/// A connection to one shared folder of a device.
pub struct ChildConnection {
    /// Rooted at the shared folder; its capabilities are those of the device.
    pub backend: BackendHandle,
    /// The device lets this contact write (Direct devices only).
    pub writable: bool,
}

/// How the tree reaches its devices. The host implements it over the
/// daemon (`ChildBackend`, `DeviceWatch`), the tests over local folders.
pub trait ChildLink: Send + Sync {
    /// Opens `root` of the device; may block while the device is reached.
    fn connect(&self, target: &PeerMountTarget, root: &BackendRoot) -> io::Result<ChildConnection>;
    /// The shared folders and connections of the device, without the root
    /// „Alle freigegebenen Bereiche“.
    fn roots(&self, target: &PeerMountTarget) -> io::Result<Vec<ChildRoot>>;
    /// Exclusive ownership of the cache `id` below `spool_root`; released
    /// when the value is dropped.
    fn acquire_lease(&self, spool_root: &Path, id: &MountId) -> io::Result<Box<dyn Any + Send>>;
}

/// What the tree takes from the drive configuration.
pub struct TreeConfig {
    pub id: MountId,
    pub metadata: MountMetadataPolicy,
    pub cache: MountCachePolicy,
    pub space: Arc<dyn CacheSpaceProbe>,
}
