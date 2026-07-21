use std::io;
use std::net::SocketAddr;
use std::process::Child;

pub(super) const MOUNT_TOKEN_ENV: &str = "SMART_EXPLORER_MOUNT_TOKEN";
pub(super) const MOUNT_IPC_ADDR_ENV: &str = "SMART_EXPLORER_MOUNT_IPC_ADDR";
pub(super) const MOUNT_CACHE_DIR_ENV: &str = "SMART_EXPLORER_MOUNT_CACHE_DIR";

pub(super) fn spawn(
    _mount_id: &crate::mount::MountId,
    _launch_token: &str,
    _ipc_addr: SocketAddr,
    _cache_root: &std::path::Path,
) -> io::Result<Child> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Virtuelle Smart-Explorer-Laufwerke werden nur unter Windows unterstuetzt",
    ))
}
